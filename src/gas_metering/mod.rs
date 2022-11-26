//! This module is used to instrument a Wasm module with the gas metering code.
//!
//! The primary public interface is the [`inject`] function which transforms a given
//! module into one that charges gas for code to be executed. See function documentation for usage
//! and details.

mod backend;

pub use backend::{host_function, mutable_global, Backend, GasMeter};

#[cfg(test)]
mod validation;

use alloc::{vec, vec::Vec};
use core::{cmp::min, mem, num::NonZeroU32};
use parity_wasm::{
	builder,
	elements::{self, IndexMap, Instruction, ValueType},
};

/// An interface that describes instruction costs.
pub trait Rules {
	/// Returns the cost for the passed `instruction`.
	///
	/// Returning `None` makes the gas instrumention end with an error. This is meant
	/// as a way to have a partial rule set where any instruction that is not specifed
	/// is considered as forbidden.
	fn instruction_cost(&self, instruction: &Instruction) -> Option<u32>;

	/// Returns the costs for growing the memory using the `memory.grow` instruction.
	///
	/// Please note that these costs are in addition to the costs specified by `instruction_cost`
	/// for the `memory.grow` instruction. Those are meant as dynamic costs which take the
	/// amount of pages that the memory is grown by into consideration. This is not possible
	/// using `instruction_cost` because those costs depend on the stack and must be injected as
	/// code into the function calling `memory.grow`. Therefore returning anything but
	/// [`MemoryGrowCost::Free`] introduces some overhead to the `memory.grow` instruction.
	fn memory_grow_cost(&self) -> MemoryGrowCost;

	/// Returns the cost of zero-inilization of a single local variable.
	fn local_init_cost(&self) -> u32;
}

/// Dynamic costs for memory growth.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum MemoryGrowCost {
	/// Skip per page charge.
	///
	/// # Note
	///
	/// This makes sense when the amount of pages that a module is allowed to use is limited
	/// to a rather small number by static validation. In that case it is viable to
	/// benchmark the costs of `memory.grow` as the worst case (growing to to the maximum
	/// number of pages).
	Free,
	/// Charge the specified amount for each page that the memory is grown by.
	Linear(NonZeroU32),
}

impl MemoryGrowCost {
	/// True iff memory growths code needs to be injected.
	fn enabled(&self) -> bool {
		match self {
			Self::Free => false,
			Self::Linear(_) => true,
		}
	}
}

/// A type that implements [`Rules`] so that every instruction costs the same.
///
/// This is a simplification that is mostly useful for development and testing.
///
/// # Note
///
/// In a production environment it usually makes no sense to assign every instruction
/// the same cost. A proper implemention of [`Rules`] should be provided that is probably
/// created by benchmarking.
pub struct ConstantCostRules {
	instruction_cost: u32,
	memory_grow_cost: u32,
	local_init_cost: u32,
}

impl ConstantCostRules {
	/// Create a new [`ConstantCostRules`].
	///
	/// Uses `instruction_cost` for every instruction and `memory_grow_cost` to dynamically
	/// meter the memory growth instruction.
	pub fn new(instruction_cost: u32, memory_grow_cost: u32, local_init_cost: u32) -> Self {
		Self { instruction_cost, memory_grow_cost, local_init_cost }
	}
}

impl Default for ConstantCostRules {
	/// Uses instruction cost of `1` and disables memory growth instrumentation.
	fn default() -> Self {
		Self { instruction_cost: 1, memory_grow_cost: 0, local_init_cost: 1 }
	}
}

impl Rules for ConstantCostRules {
	fn instruction_cost(&self, _: &Instruction) -> Option<u32> {
		Some(self.instruction_cost)
	}

	fn memory_grow_cost(&self) -> MemoryGrowCost {
		NonZeroU32::new(self.memory_grow_cost).map_or(MemoryGrowCost::Free, MemoryGrowCost::Linear)
	}

	fn local_init_cost(&self) -> u32 {
		self.local_init_cost
	}
}

/// Transforms a given module into one that tracks the gas charged during its execution.
///
/// The output module uses the `gas` function to track the gas spent. The function could be either
/// an imported or a local one modifying a mutable global. The argument is the amount of gas
/// required to continue execution. The execution engine is meant to keep track of the total amount
/// of gas used and trap or otherwise halt execution of the runtime if the gas usage exceeds some
/// allowed limit.
///
/// The body of each function of the original module is divided into metered blocks, and the calls
/// to charge gas are inserted at the beginning of every such block of code. A metered block is
/// defined so that, unless there is a trap, either all of the instructions are executed or none
/// are. These are similar to basic blocks in a control flow graph, except that in some cases
/// multiple basic blocks can be merged into a single metered block. This is the case if any path
/// through the control flow graph containing one basic block also contains another.
///
/// Charging gas at the beginning of each metered block ensures that 1) all instructions
/// executed are already paid for, 2) instructions that will not be executed are not charged for
/// unless execution traps, and 3) the number of calls to `gas` is minimized. The corollary is
/// that modules instrumented with this metering code may charge gas for instructions not
/// executed in the event of a trap.
///
/// Additionally, each `memory.grow` instruction found in the module is instrumented to first
/// make a call to charge gas for the additional pages requested. This cannot be done as part of
/// the block level gas charges as the gas cost is not static and depends on the stack argument
/// to `memory.grow`.
///
/// The above transformations are performed for every function body defined in the module. This
/// function also rewrites all function indices references by code, table elements, etc., since
/// the addition of an imported functions changes the indices of module-defined functions. If
/// the module has a `NameSection`, added by calling `parse_names`, the indices will also be
/// updated.
///
/// Syncronizing the amount of gas charged with the execution engine can be done in two ways. The
/// first way is by calling the imported `gas` host function, see [`host_function`] for details. The
/// second way is by using a local `gas` function together with a mutable global, see
/// [`mutable_global`] for details.
///
/// This routine runs in time linear in the size of the input module.
///
/// The function fails if the module contains any operation forbidden by gas rule set, returning
/// the original module as an `Err`.
pub fn inject<R: Rules, B: Backend>(
	module: elements::Module,
	backend: B,
	rules: &R,
) -> Result<elements::Module, elements::Module> {
	// Prepare module and return the gas function
	let gas_meter = backend.gas_meter(&module, rules);

	let import_count = module.import_count(elements::ImportCountType::Function) as u32;
	let functions_space = module.functions_space() as u32;
	let gas_global_idx = module.globals_space() as u32;

	let mut mbuilder = builder::from_module(module);

	// Calculate the indexes and gas function cost,
	// for external gas function the cost is counted on the host side
	let (gas_func_idx, total_func, gas_fn_cost) = match gas_meter {
		GasMeter::External { module: gas_module, function } => {
			// Inject the import of the gas function
			let import_sig = mbuilder
				.push_signature(builder::signature().with_param(ValueType::I64).build_sig());
			mbuilder.push_import(
				builder::import()
					.module(gas_module)
					.field(function)
					.external()
					.func(import_sig)
					.build(),
			);

			(import_count, functions_space + 1, 0)
		},
		GasMeter::Internal { global, ref func_instructions, cost } => {
			// Inject the gas counting global
			mbuilder.push_global(
				builder::global()
					.with_type(ValueType::I64)
					.mutable()
					.init_expr(Instruction::I64Const(0))
					.build(),
			);
			// Inject the export entry for the gas counting global
			let ebuilder = builder::ExportBuilder::new();
			let global_export = ebuilder
				.field(global)
				.with_internal(elements::Internal::Global(gas_global_idx))
				.build();
			mbuilder.push_export(global_export);

			let func_idx = functions_space;

			// Build local gas function
			let gas_func_sig =
				builder::SignatureBuilder::new().with_param(ValueType::I64).build_sig();

			let function = builder::FunctionBuilder::new()
				.with_signature(gas_func_sig)
				.body()
				.with_instructions(func_instructions.clone())
				.build()
				.build();

			// Inject local gas function
			mbuilder.push_function(function);

			(func_idx, func_idx + 1, cost)
		},
	};

	// We need the built the module for making injections to its blocks
	let mut module = mbuilder.build();

	let mut need_grow_counter = false;
	let mut error = false;

	// Iterate over module sections and perform needed transformations.
	// Indexes are needed to be fixed up in `GasMeter::External` case, as it adds an imported
	// function, which goes to the beginning of the module's functions space.
	for section in module.sections_mut() {
		match section {
			elements::Section::Code(code_section) => {
				let injection_targets = match gas_meter {
					GasMeter::External { .. } => code_section.bodies_mut().as_mut_slice(),
					// Don't inject counters to the local gas function, which is the last one as
					// it's just added. Cost for its execution is added statically before each
					// invocation (see `inject_counter()`).
					GasMeter::Internal { .. } => {
						let len = code_section.bodies().len();
						&mut code_section.bodies_mut()[..len - 1]
					},
				};

				for func_body in injection_targets {
					// Increment calling addresses if needed
					if let GasMeter::External { .. } = gas_meter {
						for instruction in func_body.code_mut().elements_mut().iter_mut() {
							if let Instruction::Call(call_index) = instruction {
								if *call_index >= gas_func_idx {
									*call_index += 1
								}
							}
						}
					}
					let locals_count = func_body
						.locals()
						.iter()
						.fold(0, |count, val_type| count + val_type.count());
					if inject_counter(
						func_body.code_mut(),
						gas_fn_cost,
						locals_count,
						rules,
						gas_func_idx,
					)
					.is_err()
					{
						error = true;
						break
					}
					if rules.memory_grow_cost().enabled() &&
						inject_grow_counter(func_body.code_mut(), total_func) > 0
					{
						need_grow_counter = true;
					}
				}
			},
			elements::Section::Export(export_section) =>
				if let GasMeter::External { module: _, function: _ } = gas_meter {
					for export in export_section.entries_mut() {
						if let elements::Internal::Function(func_index) = export.internal_mut() {
							if *func_index >= gas_func_idx {
								*func_index += 1
							}
						}
					}
				},
			elements::Section::Element(elements_section) => {
				// Note that we do not need to check the element type referenced because in the
				// WebAssembly 1.0 spec, the only allowed element type is funcref.
				if let GasMeter::External { .. } = gas_meter {
					for segment in elements_section.entries_mut() {
						// update all indirect call addresses initial values
						for func_index in segment.members_mut() {
							if *func_index >= gas_func_idx {
								*func_index += 1
							}
						}
					}
				}
			},
			elements::Section::Start(start_idx) =>
				if let GasMeter::External { .. } = gas_meter {
					if *start_idx >= gas_func_idx {
						*start_idx += 1
					}
				},
			elements::Section::Name(s) =>
				if let GasMeter::External { .. } = gas_meter {
					for functions in s.functions_mut() {
						*functions.names_mut() =
							IndexMap::from_iter(functions.names().iter().map(|(mut idx, name)| {
								if idx >= gas_func_idx {
									idx += 1;
								}

								(idx, name.clone())
							}));
					}
				},
			_ => {},
		}
	}

	if error {
		return Err(module)
	}

	if need_grow_counter {
		Ok(add_grow_counter(module, rules, gas_func_idx))
	} else {
		Ok(module)
	}
}

/// A control flow block is opened with the `block`, `loop`, and `if` instructions and is closed
/// with `end`. Each block implicitly defines a new label. The control blocks form a stack during
/// program execution.
///
/// An example of block:
///
/// ```ignore
/// loop
///   i32.const 1
///   get_local 0
///   i32.sub
///   tee_local 0
///   br_if 0
/// end
/// ```
///
/// The start of the block is `i32.const 1`.
#[derive(Debug)]
struct ControlBlock {
	/// The lowest control stack index corresponding to a forward jump targeted by a br, br_if, or
	/// br_table instruction within this control block. The index must refer to a control block
	/// that is not a loop, meaning it is a forward jump. Given the way Wasm control flow is
	/// structured, the lowest index on the stack represents the furthest forward branch target.
	///
	/// This value will always be at most the index of the block itself, even if there is no
	/// explicit br instruction targeting this control block. This does not affect how the value is
	/// used in the metering algorithm.
	lowest_forward_br_target: usize,

	/// The active metering block that new instructions contribute a gas cost towards.
	active_metered_block: MeteredBlock,

	/// Whether the control block is a loop. Loops have the distinguishing feature that branches to
	/// them jump to the beginning of the block, not the end as with the other control blocks.
	is_loop: bool,
}

/// A block of code that metering instructions will be inserted at the beginning of. Metered blocks
/// are constructed with the property that, in the absence of any traps, either all instructions in
/// the block are executed or none are.
#[derive(Debug)]
struct MeteredBlock {
	/// Index of the first instruction (aka `Opcode`) in the block.
	start_pos: usize,
	/// Sum of costs of all instructions until end of the block.
	cost: u64,
}

/// Counter is used to manage state during the gas metering algorithm implemented by
/// `inject_counter`.
struct Counter {
	/// A stack of control blocks. This stack grows when new control blocks are opened with
	/// `block`, `loop`, and `if` and shrinks when control blocks are closed with `end`. The first
	/// block on the stack corresponds to the function body, not to any labelled block. Therefore
	/// the actual Wasm label index associated with each control block is 1 less than its position
	/// in this stack.
	stack: Vec<ControlBlock>,

	/// A list of metered blocks that have been finalized, meaning they will no longer change.
	finalized_blocks: Vec<MeteredBlock>,
}

impl Counter {
	fn new() -> Counter {
		Counter { stack: Vec::new(), finalized_blocks: Vec::new() }
	}

	/// Open a new control block. The cursor is the position of the first instruction in the block.
	fn begin_control_block(&mut self, cursor: usize, is_loop: bool) {
		let index = self.stack.len();
		self.stack.push(ControlBlock {
			lowest_forward_br_target: index,
			active_metered_block: MeteredBlock { start_pos: cursor, cost: 0 },
			is_loop,
		})
	}

	/// Close the last control block. The cursor is the position of the final (pseudo-)instruction
	/// in the block.
	fn finalize_control_block(&mut self, cursor: usize) -> Result<(), ()> {
		// This either finalizes the active metered block or merges its cost into the active
		// metered block in the previous control block on the stack.
		self.finalize_metered_block(cursor)?;

		// Pop the control block stack.
		let closing_control_block = self.stack.pop().ok_or(())?;
		let closing_control_index = self.stack.len();

		if self.stack.is_empty() {
			return Ok(())
		}

		// Update the lowest_forward_br_target for the control block now on top of the stack.
		{
			let control_block = self.stack.last_mut().ok_or(())?;
			control_block.lowest_forward_br_target = min(
				control_block.lowest_forward_br_target,
				closing_control_block.lowest_forward_br_target,
			);
		}

		// If there may have been a branch to a lower index, then also finalize the active metered
		// block for the previous control block. Otherwise, finalize it and begin a new one.
		let may_br_out = closing_control_block.lowest_forward_br_target < closing_control_index;
		if may_br_out {
			self.finalize_metered_block(cursor)?;
		}

		Ok(())
	}

	/// Finalize the current active metered block.
	///
	/// Finalized blocks have final cost which will not change later.
	fn finalize_metered_block(&mut self, cursor: usize) -> Result<(), ()> {
		let closing_metered_block = {
			let control_block = self.stack.last_mut().ok_or(())?;
			mem::replace(
				&mut control_block.active_metered_block,
				MeteredBlock { start_pos: cursor + 1, cost: 0 },
			)
		};

		// If the block was opened with a `block`, then its start position will be set to that of
		// the active metered block in the control block one higher on the stack. This is because
		// any instructions between a `block` and the first branch are part of the same basic block
		// as the preceding instruction. In this case, instead of finalizing the block, merge its
		// cost into the other active metered block to avoid injecting unnecessary instructions.
		let last_index = self.stack.len() - 1;
		if last_index > 0 {
			let prev_control_block = self
				.stack
				.get_mut(last_index - 1)
				.expect("last_index is greater than 0; last_index is stack size - 1; qed");
			let prev_metered_block = &mut prev_control_block.active_metered_block;
			if closing_metered_block.start_pos == prev_metered_block.start_pos {
				prev_metered_block.cost =
					prev_metered_block.cost.checked_add(closing_metered_block.cost).ok_or(())?;
				return Ok(())
			}
		}

		if closing_metered_block.cost > 0 {
			self.finalized_blocks.push(closing_metered_block);
		}
		Ok(())
	}

	/// Handle a branch instruction in the program. The cursor is the index of the branch
	/// instruction in the program. The indices are the stack positions of the target control
	/// blocks. Recall that the index is 0 for a `return` and relatively indexed from the top of
	/// the stack by the label of `br`, `br_if`, and `br_table` instructions.
	fn branch(&mut self, cursor: usize, indices: &[usize]) -> Result<(), ()> {
		self.finalize_metered_block(cursor)?;

		// Update the lowest_forward_br_target of the current control block.
		for &index in indices {
			let target_is_loop = {
				let target_block = self.stack.get(index).ok_or(())?;
				target_block.is_loop
			};
			if target_is_loop {
				continue
			}

			let control_block = self.stack.last_mut().ok_or(())?;
			control_block.lowest_forward_br_target =
				min(control_block.lowest_forward_br_target, index);
		}

		Ok(())
	}

	/// Returns the stack index of the active control block. Returns None if stack is empty.
	fn active_control_block_index(&self) -> Option<usize> {
		self.stack.len().checked_sub(1)
	}

	/// Get a reference to the currently active metered block.
	fn active_metered_block(&mut self) -> Result<&mut MeteredBlock, ()> {
		let top_block = self.stack.last_mut().ok_or(())?;
		Ok(&mut top_block.active_metered_block)
	}

	/// Increment the cost of the current block by the specified value.
	fn increment(&mut self, val: u32) -> Result<(), ()> {
		let top_block = self.active_metered_block()?;
		top_block.cost = top_block.cost.checked_add(val.into()).ok_or(())?;
		Ok(())
	}
}

fn inject_grow_counter(instructions: &mut elements::Instructions, grow_counter_func: u32) -> usize {
	use parity_wasm::elements::Instruction::*;
	let mut counter = 0;
	for instruction in instructions.elements_mut() {
		if let GrowMemory(_) = *instruction {
			*instruction = Call(grow_counter_func);
			counter += 1;
		}
	}
	counter
}

fn add_grow_counter<R: Rules>(
	module: elements::Module,
	rules: &R,
	gas_func: u32,
) -> elements::Module {
	use parity_wasm::elements::Instruction::*;

	let cost = match rules.memory_grow_cost() {
		MemoryGrowCost::Free => return module,
		MemoryGrowCost::Linear(val) => val.get(),
	};

	let mut b = builder::from_module(module);
	b.push_function(
		builder::function()
			.signature()
			.with_param(ValueType::I32)
			.with_result(ValueType::I32)
			.build()
			.body()
			.with_instructions(elements::Instructions::new(vec![
				GetLocal(0),
				GetLocal(0),
				I64ExtendUI32,
				I64Const(i64::from(cost)),
				I64Mul,
				// todo: there should be strong guarantee that it does not return anything on
				// stack?
				Call(gas_func),
				GrowMemory(0),
				End,
			]))
			.build()
			.build(),
	);

	b.build()
}

fn determine_metered_blocks<R: Rules>(
	instructions: &elements::Instructions,
	rules: &R,
) -> Result<Vec<MeteredBlock>, ()> {
	use parity_wasm::elements::Instruction::*;

	let mut counter = Counter::new();

	// Begin an implicit function (i.e. `func...end`) block.
	counter.begin_control_block(0, false);

	for cursor in 0..instructions.elements().len() {
		let instruction = &instructions.elements()[cursor];
		let instruction_cost = rules.instruction_cost(instruction).ok_or(())?;
		match instruction {
			Block(_) => {
				counter.increment(instruction_cost)?;

				// Begin new block. The cost of the following opcodes until `end` or `else` will
				// be included into this block. The start position is set to that of the previous
				// active metered block to signal that they should be merged in order to reduce
				// unnecessary metering instructions.
				let top_block_start_pos = counter.active_metered_block()?.start_pos;
				counter.begin_control_block(top_block_start_pos, false);
			},
			If(_) => {
				counter.increment(instruction_cost)?;
				counter.begin_control_block(cursor + 1, false);
			},
			Loop(_) => {
				counter.increment(instruction_cost)?;
				counter.begin_control_block(cursor + 1, true);
			},
			End => {
				counter.finalize_control_block(cursor)?;
			},
			Else => {
				counter.finalize_metered_block(cursor)?;
			},
			Br(label) | BrIf(label) => {
				counter.increment(instruction_cost)?;

				// Label is a relative index into the control stack.
				let active_index = counter.active_control_block_index().ok_or(())?;
				let target_index = active_index.checked_sub(*label as usize).ok_or(())?;
				counter.branch(cursor, &[target_index])?;
			},
			BrTable(br_table_data) => {
				counter.increment(instruction_cost)?;

				let active_index = counter.active_control_block_index().ok_or(())?;
				let target_indices = [br_table_data.default]
					.iter()
					.chain(br_table_data.table.iter())
					.map(|label| active_index.checked_sub(*label as usize))
					.collect::<Option<Vec<_>>>()
					.ok_or(())?;
				counter.branch(cursor, &target_indices)?;
			},
			Return => {
				counter.increment(instruction_cost)?;
				counter.branch(cursor, &[0])?;
			},
			_ => {
				// An ordinal non control flow instruction increments the cost of the current block.
				counter.increment(instruction_cost)?;
			},
		}
	}

	counter.finalized_blocks.sort_unstable_by_key(|block| block.start_pos);
	Ok(counter.finalized_blocks)
}

fn inject_counter<R: Rules>(
	instructions: &mut elements::Instructions,
	gas_function_cost: u64,
	locals_count: u32,
	rules: &R,
	gas_func: u32,
) -> Result<(), ()> {
	let blocks = determine_metered_blocks(instructions, rules)?;
	let locals_init_cost =
		(rules.local_init_cost() as u64).checked_mul(locals_count as u64).ok_or(())?;
	insert_metering_calls(instructions, gas_function_cost, locals_init_cost, blocks, gas_func)
}

// Then insert metering calls into a sequence of instructions given the block locations and costs.
fn insert_metering_calls(
	instructions: &mut elements::Instructions,
	gas_function_cost: u64,
	locals_init_cost: u64,
	blocks: Vec<MeteredBlock>,
	gas_func: u32,
) -> Result<(), ()> {
	use parity_wasm::elements::Instruction::*;

	// To do this in linear time, construct a new vector of instructions, copying over old
	// instructions one by one and injecting new ones as required.
	let new_instrs_len = instructions.elements().len() + 2 * blocks.len();
	let original_instrs =
		mem::replace(instructions.elements_mut(), Vec::with_capacity(new_instrs_len));
	let new_instrs = instructions.elements_mut();

	let mut block_iter = blocks.into_iter().peekable();
	for (original_pos, instr) in original_instrs.into_iter().enumerate() {
		// If there the next block starts at this position, inject metering instructions.
		let used_block = if let Some(block) = block_iter.peek() {
			if block.start_pos == original_pos {
				// Charge locals initialization cost to the beginning of the function block.
				let block_cost = if original_pos == 0 {
					block.cost.checked_add(locals_init_cost).ok_or(())?
				} else {
					block.cost
				};

				new_instrs
					.push(I64Const((block_cost.checked_add(gas_function_cost).ok_or(())?) as i64));
				new_instrs.push(Call(gas_func));
				true
			} else {
				false
			}
		} else {
			false
		};

		if used_block {
			block_iter.next();
		}

		// Copy over the original instruction.
		new_instrs.push(instr);
	}

	if block_iter.next().is_some() {
		return Err(())
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use parity_wasm::{builder, elements, elements::Instruction::*, serialize};
	use pretty_assertions::assert_eq;

	fn get_function_body(
		module: &elements::Module,
		index: usize,
	) -> Option<&[elements::Instruction]> {
		module
			.code_section()
			.and_then(|code_section| code_section.bodies().get(index))
			.map(|func_body| func_body.code().elements())
	}

	#[test]
	fn simple_grow_host_fn() {
		let module = parse_wat(
			r#"(module
			(func (result i32)
			  global.get 0
			  memory.grow)
			(global i32 (i32.const 42))
			(memory 0 1)
			)"#,
		);
		let backend = host_function::Injector::new("env", "gas");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::new(1, 10_000, 1)).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 0).unwrap(),
			&vec![I64Const(2), Call(0), GetGlobal(0), Call(2), End][..]
		);
		assert_eq!(
			get_function_body(&injected_module, 1).unwrap(),
			&vec![
				GetLocal(0),
				GetLocal(0),
				I64ExtendUI32,
				I64Const(10000),
				I64Mul,
				Call(0),
				GrowMemory(0),
				End,
			][..]
		);

		let binary = serialize(injected_module).expect("serialization failed");
		wasmparser::validate(&binary).unwrap();
	}

	#[test]
	fn simple_grow_mut_global() {
		let module = parse_wat(
			r#"(module
			(func (result i32)
			  global.get 0
			  memory.grow)
			(global i32 (i32.const 42))
			(memory 0 1)
			)"#,
		);
		let backend = mutable_global::Injector::new("gas_left");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::new(1, 10_000, 1)).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 0).unwrap(),
			&vec![I64Const(13), Call(1), GetGlobal(0), Call(2), End][..]
		);
		assert_eq!(
			get_function_body(&injected_module, 1).unwrap(),
			&vec![
				Instruction::GetGlobal(1),
				Instruction::GetLocal(0),
				Instruction::I64GeU,
				Instruction::If(elements::BlockType::NoResult),
				Instruction::GetGlobal(1),
				Instruction::GetLocal(0),
				Instruction::I64Sub,
				Instruction::SetGlobal(1),
				Instruction::Else,
				// sentinel val u64::MAX
				Instruction::I64Const(-1i64), // non-charged instruction
				Instruction::SetGlobal(1),    // non-charged instruction
				Instruction::Unreachable,     // non-charged instruction
				Instruction::End,
				Instruction::End,
			][..]
		);
		assert_eq!(
			get_function_body(&injected_module, 2).unwrap(),
			&vec![
				GetLocal(0),
				GetLocal(0),
				I64ExtendUI32,
				I64Const(10000),
				I64Mul,
				Call(1),
				GrowMemory(0),
				End,
			][..]
		);

		let binary = serialize(injected_module).expect("serialization failed");
		wasmparser::validate(&binary).unwrap();
	}

	#[test]
	fn grow_no_gas_no_track_host_fn() {
		let module = parse_wat(
			r"(module
			(func (result i32)
			  global.get 0
			  memory.grow)
			(global i32 (i32.const 42))
			(memory 0 1)
			)",
		);
		let backend = host_function::Injector::new("env", "gas");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::default()).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 0).unwrap(),
			&vec![I64Const(2), Call(0), GetGlobal(0), GrowMemory(0), End][..]
		);

		assert_eq!(injected_module.functions_space(), 2);

		let binary = serialize(injected_module).expect("serialization failed");
		wasmparser::validate(&binary).unwrap();
	}

	#[test]
	fn grow_no_gas_no_track_mut_global() {
		let module = parse_wat(
			r"(module
			(func (result i32)
			  global.get 0
			  memory.grow)
			(global i32 (i32.const 42))
			(memory 0 1)
			)",
		);
		let backend = mutable_global::Injector::new("gas_left");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::default()).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 0).unwrap(),
			&vec![I64Const(13), Call(1), GetGlobal(0), GrowMemory(0), End][..]
		);

		assert_eq!(injected_module.functions_space(), 2);

		let binary = serialize(injected_module).expect("serialization failed");
		wasmparser::validate(&binary).unwrap();
	}

	#[test]
	fn call_index_host_fn() {
		let module = builder::module()
			.global()
			.value_type()
			.i32()
			.build()
			.function()
			.signature()
			.param()
			.i32()
			.build()
			.body()
			.build()
			.build()
			.function()
			.signature()
			.param()
			.i32()
			.build()
			.body()
			.with_instructions(elements::Instructions::new(vec![
				Call(0),
				If(elements::BlockType::NoResult),
				Call(0),
				Call(0),
				Call(0),
				Else,
				Call(0),
				Call(0),
				End,
				Call(0),
				End,
			]))
			.build()
			.build()
			.build();

		let backend = host_function::Injector::new("env", "gas");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::default()).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 1).unwrap(),
			&vec![
				I64Const(3),
				Call(0),
				Call(1),
				If(elements::BlockType::NoResult),
				I64Const(3),
				Call(0),
				Call(1),
				Call(1),
				Call(1),
				Else,
				I64Const(2),
				Call(0),
				Call(1),
				Call(1),
				End,
				Call(1),
				End
			][..]
		);
	}

	#[test]
	fn call_index_mut_global() {
		let module = builder::module()
			.global()
			.value_type()
			.i32()
			.build()
			.function()
			.signature()
			.param()
			.i32()
			.build()
			.body()
			.build()
			.build()
			.function()
			.signature()
			.param()
			.i32()
			.build()
			.body()
			.with_instructions(elements::Instructions::new(vec![
				Call(0),
				If(elements::BlockType::NoResult),
				Call(0),
				Call(0),
				Call(0),
				Else,
				Call(0),
				Call(0),
				End,
				Call(0),
				End,
			]))
			.build()
			.build()
			.build();

		let backend = mutable_global::Injector::new("gas_left");
		let injected_module =
			super::inject(module, backend, &ConstantCostRules::default()).unwrap();

		assert_eq!(
			get_function_body(&injected_module, 1).unwrap(),
			&vec![
				I64Const(14),
				Call(2),
				Call(0),
				If(elements::BlockType::NoResult),
				I64Const(14),
				Call(2),
				Call(0),
				Call(0),
				Call(0),
				Else,
				I64Const(13),
				Call(2),
				Call(0),
				Call(0),
				End,
				Call(0),
				End
			][..]
		);
	}

	fn parse_wat(source: &str) -> elements::Module {
		let module_bytes = wat::parse_str(source).unwrap();
		elements::deserialize_buffer(module_bytes.as_ref()).unwrap()
	}

	macro_rules! test_gas_counter_injection {
		(names = ($name1:ident, $name2:ident); input = $input:expr; expected = $expected:expr) => {
			#[test]
			fn $name1() {
				let input_module = parse_wat($input);
				let expected_module = parse_wat($expected);
				let injected_module = super::inject(
					input_module,
					host_function::Injector::new("env", "gas"),
					&ConstantCostRules::default(),
				)
				.expect("inject_gas_counter call failed");

				let actual_func_body = get_function_body(&injected_module, 0)
					.expect("injected module must have a function body");
				let expected_func_body = get_function_body(&expected_module, 0)
					.expect("post-module must have a function body");

				assert_eq!(actual_func_body, expected_func_body);
			}

			#[test]
			fn $name2() {
				let input_module = parse_wat($input);
				let draft_module = parse_wat($expected);
				let gas_fun_cost = match mutable_global::Injector::new("gas_left")
					.gas_meter(&input_module, &ConstantCostRules::default())
				{
					GasMeter::Internal { cost, .. } => cost as i64,
					_ => 0i64,
				};

				let injected_module = super::inject(
					input_module,
					mutable_global::Injector::new("gas_left"),
					&ConstantCostRules::default(),
				)
				.expect("inject_gas_counter call failed");

				let actual_func_body = get_function_body(&injected_module, 0)
					.expect("injected module must have a function body");
				let mut expected_func_body = get_function_body(&draft_module, 0)
					.expect("post-module must have a function body")
					.to_vec();

				// modify expected instructions set for gas_metering::mutable_global
				let mut iter = expected_func_body.iter_mut();
				while let Some(ins) = iter.next() {
					if let I64Const(cost) = ins {
						if let Some(ins_next) = iter.next() {
							if let Call(0) = ins_next {
								*cost += gas_fun_cost;
								*ins_next = Call(1);
							}
						}
					}
				}

				assert_eq!(actual_func_body, &expected_func_body);
			}
		};
	}

	test_gas_counter_injection! {
		names = (simple_host_fn, simple_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 1))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (nested_host_fn, nested_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(block
					(get_global 0)
					(get_global 0)
					(get_global 0))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 6))
				(get_global 0)
				(block
					(get_global 0)
					(get_global 0)
					(get_global 0))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (ifelse_host_fn, ifelse_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(if
					(then
						(get_global 0)
						(get_global 0)
						(get_global 0))
					(else
						(get_global 0)
						(get_global 0)))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 3))
				(get_global 0)
				(if
					(then
						(call 0 (i64.const 3))
						(get_global 0)
						(get_global 0)
						(get_global 0))
					(else
						(call 0 (i64.const 2))
						(get_global 0)
						(get_global 0)))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (branch_innermost_host_fn, branch_innermost_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(block
					(get_global 0)
					(drop)
					(br 0)
					(get_global 0)
					(drop))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 6))
				(get_global 0)
				(block
					(get_global 0)
					(drop)
					(br 0)
					(call 0 (i64.const 2))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (branch_outer_block_host_fn, branch_outer_block_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(block
					(get_global 0)
					(if
						(then
							(get_global 0)
							(get_global 0)
							(drop)
							(br_if 1)))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 5))
				(get_global 0)
				(block
					(get_global 0)
					(if
						(then
							(call 0 (i64.const 4))
							(get_global 0)
							(get_global 0)
							(drop)
							(br_if 1)))
					(call 0 (i64.const 2))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (branch_outer_loop_host_fn, branch_outer_loop_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(loop
					(get_global 0)
					(if
						(then
							(get_global 0)
							(br_if 0))
						(else
							(get_global 0)
							(get_global 0)
							(drop)
							(br_if 1)))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 3))
				(get_global 0)
				(loop
					(call 0 (i64.const 4))
					(get_global 0)
					(if
						(then
							(call 0 (i64.const 2))
							(get_global 0)
							(br_if 0))
						(else
							(call 0 (i64.const 4))
							(get_global 0)
							(get_global 0)
							(drop)
							(br_if 1)))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (return_from_func_host_fn, return_from_func_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(if
					(then
						(return)))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 2))
				(get_global 0)
				(if
					(then
						(call 0 (i64.const 1))
						(return)))
				(call 0 (i64.const 1))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (branch_from_if_not_else_host_fn, branch_from_if_not_else_mut_global);
		input = r#"
		(module
			(func (result i32)
				(get_global 0)
				(block
					(get_global 0)
					(if
						(then (br 1))
						(else (br 0)))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#;
		expected = r#"
		(module
			(func (result i32)
				(call 0 (i64.const 5))
				(get_global 0)
				(block
					(get_global 0)
					(if
						(then
							(call 0 (i64.const 1))
							(br 1))
						(else
							(call 0 (i64.const 1))
							(br 0)))
					(call 0 (i64.const 2))
					(get_global 0)
					(drop))
				(get_global 0)))
		"#
	}

	test_gas_counter_injection! {
		names = (empty_loop_host_fn, empty_loop_mut_global);
		input = r#"
		(module
			(func
				(loop
					(br 0)
				)
				unreachable
			)
		)
		"#;
		expected = r#"
		(module
			(func
				(call 0 (i64.const 2))
				(loop
					(call 0 (i64.const 1))
					(br 0)
				)
				unreachable
			)
		)
		"#
	}
}
