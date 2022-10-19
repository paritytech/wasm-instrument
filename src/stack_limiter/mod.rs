//! Contains the code for the stack height limiter instrumentation.

use alloc::{vec, vec::Vec};
use core::mem;
use parity_wasm::{
	builder,
	elements::{self, Instruction, Instructions, Type},
};

/// Macro to generate preamble and postamble.
macro_rules! instrument_call {
	($callee_idx: expr, $callee_stack_cost: expr, $stack_height_global_idx: expr, $stack_limit: expr) => {{
		use $crate::parity_wasm::elements::Instruction::*;
		[
			// stack_height += stack_cost(F)
			GetGlobal($stack_height_global_idx),
			I32Const($callee_stack_cost),
			I32Add,
			SetGlobal($stack_height_global_idx),
			// if stack_counter > LIMIT: unreachable
			GetGlobal($stack_height_global_idx),
			I32Const($stack_limit as i32),
			I32GtU,
			If(elements::BlockType::NoResult),
			Unreachable,
			End,
			// Original call
			Call($callee_idx),
			// stack_height -= stack_cost(F)
			GetGlobal($stack_height_global_idx),
			I32Const($callee_stack_cost),
			I32Sub,
			SetGlobal($stack_height_global_idx),
		]
	}};
}

mod max_height;
mod thunk;

pub struct Context {
	/// Number of functions that the module imports. Required to convert defined functions indicies
	/// into the global function index space.
	func_imports: u32,
	/// For each function in the function space this vector stores the respective type index.
	func_types: Vec<u32>,
	/// The index of the global variable that contains the current stack height.
	stack_height_global_idx: u32,
	/// Logical stack costs for each function in the function space. Imported functions have cost
	/// of 0.
	func_stack_costs: Vec<u32>,
	stack_limit: u32,
}

impl Context {
	/// Returns index in a global index space of a stack_height global variable.
	fn stack_height_global_idx(&self) -> u32 {
		self.stack_height_global_idx
	}

	/// Returns `stack_cost` for `func_idx`.
	fn stack_cost(&self, func_idx: u32) -> Option<u32> {
		self.func_stack_costs.get(func_idx as usize).cloned()
	}

	/// Returns a reference to the function type index given by the index into the function space.
	fn func_type(&self, func_idx: u32) -> Option<u32> {
		self.func_types.get(func_idx as usize).copied()
	}

	/// Returns stack limit specified by the rules.
	fn stack_limit(&self) -> u32 {
		self.stack_limit
	}
}

/// Inject the instumentation that makes stack overflows deterministic, by introducing
/// an upper bound of the stack size.
///
/// This pass introduces a global mutable variable to track stack height,
/// and instruments all calls with preamble and postamble.
///
/// Stack height is increased prior the call. Otherwise, the check would
/// be made after the stack frame is allocated.
///
/// The preamble is inserted before the call. It increments
/// the global stack height variable with statically determined "stack cost"
/// of the callee. If after the increment the stack height exceeds
/// the limit (specified by the `rules`) then execution traps.
/// Otherwise, the call is executed.
///
/// The postamble is inserted after the call. The purpose of the postamble is to decrease
/// the stack height by the "stack cost" of the callee function.
///
/// Note, that we can't instrument all possible ways to return from the function. The simplest
/// example would be a trap issued by the host function.
/// That means stack height global won't be equal to zero upon the next execution after such trap.
///
/// # Thunks
///
/// Because stack height is increased prior the call few problems arises:
///
/// - Stack height isn't increased upon an entry to the first function, i.e. exported function.
/// - Start function is executed externally (similar to exported functions).
/// - It is statically unknown what function will be invoked in an indirect call.
///
/// The solution for this problems is to generate a intermediate functions, called 'thunks', which
/// will increase before and decrease the stack height after the call to original function, and
/// then make exported function and table entries, start section to point to a corresponding thunks.
///
/// # Stack cost
///
/// Stack cost of the function is calculated as a sum of it's locals
/// and the maximal height of the value stack.
///
/// All values are treated equally, as they have the same size.
///
/// The rationale is that this makes it possible to use the following very naive wasm executor:
///
/// - values are implemented by a union, so each value takes a size equal to the size of the largest
///   possible value type this union can hold. (In MVP it is 8 bytes)
/// - each value from the value stack is placed on the native stack.
/// - each local variable and function argument is placed on the native stack.
/// - arguments pushed by the caller are copied into callee stack rather than shared between the
///   frames.
/// - upon entry into the function entire stack frame is allocated.
pub fn inject(
	mut module: elements::Module,
	stack_limit: u32,
) -> Result<elements::Module, &'static str> {
	let mut ctx = prepare_context(&module, stack_limit)?;

	generate_stack_height_global(&mut ctx.stack_height_global_idx, &mut module)?;
	instrument_functions(&ctx, &mut module)?;
	let module = thunk::generate_thunks(&mut ctx, module)?;

	Ok(module)
}

fn prepare_context(module: &elements::Module, stack_limit: u32) -> Result<Context, &'static str> {
	let mut ctx = Context {
		func_imports: module.import_count(elements::ImportCountType::Function) as u32,
		func_types: Vec::new(),
		stack_height_global_idx: 0,
		func_stack_costs: Vec::new(),
		stack_limit,
	};
	collect_func_types(&mut ctx, &module)?;
	compute_stack_costs(&mut ctx, &module)?;
	Ok(ctx)
}

fn collect_func_types(ctx: &mut Context, module: &elements::Module) -> Result<(), &'static str> {
	let types = module.type_section().map(|ts| ts.types()).unwrap_or(&[]);
	let functions = module.function_section().map(|fs| fs.entries()).unwrap_or(&[]);
	let imports = module.import_section().map(|is| is.entries()).unwrap_or(&[]);

	let ensure_ty = |sig_idx: u32| -> Result<(), &'static str> {
		let Type::Function(_) = types
			.get(sig_idx as usize)
			.ok_or("The signature as specified by a function isn't defined")?;
		Ok(())
	};

	for import in imports {
		if let elements::External::Function(sig_idx) = import.external() {
			ensure_ty(*sig_idx)?;
			ctx.func_types.push(*sig_idx);
		}
	}
	for def_func_idx in functions {
		ensure_ty(def_func_idx.type_ref())?;
		ctx.func_types.push(def_func_idx.type_ref());
	}

	Ok(())
}

/// Calculate stack costs for all functions in the function space.
///
/// The function space consists of the imported functions followed by defined functions.
/// All imported functions assumed to have the cost of 0.
fn compute_stack_costs(ctx: &mut Context, module: &elements::Module) -> Result<(), &'static str> {
	for _ in 0..ctx.func_imports {
		ctx.func_stack_costs.push(0);
	}
	let def_func_n = module.function_section().map(|fs| fs.entries().len()).unwrap_or(0) as u32;
	for def_func_idx in 0..def_func_n {
		let cost = compute_stack_cost(def_func_idx, ctx, module)?;
		ctx.func_stack_costs.push(cost);
	}
	Ok(())
}

/// Computes the stack cost of a given function. The function is specified by its index in the
/// declared function space.
///
/// Stack cost of a given function is the sum of it's locals count (that is,
/// number of arguments plus number of local variables) and the maximal stack
/// height.
fn compute_stack_cost(
	def_func_idx: u32,
	ctx: &Context,
	module: &elements::Module,
) -> Result<u32, &'static str> {
	let code_section =
		module.code_section().ok_or("Due to validation code section should exists")?;
	let body = &code_section
		.bodies()
		.get(def_func_idx as usize)
		.ok_or("Function body is out of bounds")?;

	let mut locals_count: u32 = 0;
	for local_group in body.locals() {
		locals_count =
			locals_count.checked_add(local_group.count()).ok_or("Overflow in local count")?;
	}

	let max_stack_height = max_height::compute(def_func_idx, ctx, module)?;

	locals_count
		.checked_add(max_stack_height)
		.ok_or("Overflow in adding locals_count and max_stack_height")
}

/// Generate a new global that will be used for tracking current stack height.
fn generate_stack_height_global(
	stack_height_global_idx: &mut u32,
	module: &mut elements::Module,
) -> Result<(), &'static str> {
	let global_entry = builder::global()
		.value_type()
		.i32()
		.mutable()
		.init_expr(Instruction::I32Const(0))
		.build();

	// Try to find an existing global section.
	for section in module.sections_mut() {
		if let elements::Section::Global(gs) = section {
			gs.entries_mut().push(global_entry);
			*stack_height_global_idx = (gs.entries().len() as u32) - 1;
			return Ok(());
		}
	}

	// Existing section not found, create one!
	//
	// It's a bit tricky since the sections have a strict prescribed order.
	let global_section = elements::GlobalSection::with_entries(vec![global_entry]);
	let prec_index = module
		.sections()
		.iter()
		.rposition(|section| {
			use elements::Section::*;
			match section {
				Type(_) | Import(_) | Function(_) | Table(_) | Memory(_) => true,
				_ => false,
			}
		})
		.ok_or("generate stack height global hasn't found any preceding sections")?;
	// now `prec_index` points to the last section preceding the `global_section`. It's guaranteed that at least
	// one of those functions is present. Therefore, the candidate position for the global section is the following
	// one. However, technically, custom sections could occupy any place between the well-known sections.
	//
	// Now, regarding `+1` here. `insert` panics iff `index > len`. `prec_index + 1` can only be equal to `len`.
	module
		.sections_mut()
		.insert(prec_index + 1, elements::Section::Global(global_section));
	// First entry in the brand new globals section.
	*stack_height_global_idx = 0;

	Ok(())
}

fn instrument_functions(ctx: &Context, module: &mut elements::Module) -> Result<(), &'static str> {
	for section in module.sections_mut() {
		if let elements::Section::Code(code_section) = section {
			for func_body in code_section.bodies_mut() {
				let opcodes = func_body.code_mut();
				instrument_function(ctx, opcodes)?;
			}
		}
	}
	Ok(())
}

/// This function searches `call` instructions and wrap each call
/// with preamble and postamble.
///
/// Before:
///
/// ```text
/// get_local 0
/// get_local 1
/// call 228
/// drop
/// ```
///
/// After:
///
/// ```text
/// get_local 0
/// get_local 1
///
/// < ... preamble ... >
///
/// call 228
///
/// < .. postamble ... >
///
/// drop
/// ```
fn instrument_function(ctx: &Context, func: &mut Instructions) -> Result<(), &'static str> {
	use Instruction::*;

	struct InstrumentCall {
		offset: usize,
		callee: u32,
		cost: u32,
	}

	let calls: Vec<_> = func
		.elements()
		.iter()
		.enumerate()
		.filter_map(|(offset, instruction)| {
			if let Call(callee) = instruction {
				ctx.stack_cost(*callee).and_then(|cost| {
					if cost > 0 {
						Some(InstrumentCall { callee: *callee, offset, cost })
					} else {
						None
					}
				})
			} else {
				None
			}
		})
		.collect();

	// The `instrumented_call!` contains the call itself. This is why we need to subtract one.
	let len = func.elements().len() + calls.len() * (instrument_call!(0, 0, 0, 0).len() - 1);
	let original_instrs = mem::replace(func.elements_mut(), Vec::with_capacity(len));
	let new_instrs = func.elements_mut();

	let mut calls = calls.into_iter().peekable();
	for (original_pos, instr) in original_instrs.into_iter().enumerate() {
		// whether there is some call instruction at this position that needs to be instrumented
		let did_instrument = if let Some(call) = calls.peek() {
			if call.offset == original_pos {
				let new_seq = instrument_call!(
					call.callee,
					call.cost as i32,
					ctx.stack_height_global_idx(),
					ctx.stack_limit()
				);
				new_instrs.extend_from_slice(&new_seq);
				true
			} else {
				false
			}
		} else {
			false
		};

		if did_instrument {
			calls.next();
		} else {
			new_instrs.push(instr);
		}
	}

	if calls.next().is_some() {
		return Err("Not all calls were used")
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use parity_wasm::elements;

	fn parse_wat(source: &str) -> elements::Module {
		elements::deserialize_buffer(&wat::parse_str(source).expect("Failed to wat2wasm"))
			.expect("Failed to deserialize the module")
	}

	fn validate_module(module: elements::Module) {
		let binary = elements::serialize(module).expect("Failed to serialize");
		wasmparser::validate(&binary).expect("Invalid module");
	}

	#[test]
	fn test_with_params_and_result() {
		let module = parse_wat(
			r#"
(module
	(func (export "i32.add") (param i32 i32) (result i32)
		get_local 0
	get_local 1
	i32.add
	)
)
"#,
		);

		let module = inject(module, 1024).expect("Failed to inject stack counter");
		validate_module(module);
	}
}
