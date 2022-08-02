use super::resolve_func_type;
use alloc::{vec, vec::Vec};
use parity_wasm::elements::{self, BlockType, Type, ValueType};

#[cfg(feature = "sign_ext")]
use parity_wasm::elements::SignExtInstruction;

#[cfg(feature = "trace-log")]
macro_rules! trace {
    ($($tt:tt)*) => {
      ::log::trace!($($tt)*);
    };
}

#[cfg(not(feature = "trace-log"))]
macro_rules! trace {
	($($tt:tt)*) => {};
}

// The cost in stack items that should be charged per call of a function. This is
// is a static cost that is added to each function call. This makes sense because even
// if a function does not use any parameters or locals some stack space on the host
// machine might be consumed to hold some context.
const ACTIVATION_FRAME_HEIGHT: u32 = 2;

// Weight of an activation frame.
const ACTIVATION_FRAME_WEIGHT: u32 = 32;

/// Control stack frame.
#[derive(Debug)]
struct Frame {
	/// Stack becomes polymorphic only after an instruction that
	/// never passes control further was executed.
	is_polymorphic: bool,
	unreachable_depth: u32,

	/// Type of value which will be pushed after exiting
	/// the current block or `None` if block does not return a result.
	result_type: Option<ValueType>,

	/// Type of value which should be poped upon a branch to
	/// this frame or `None` if branching shouldn't affect the stack.
	///
	/// This might be diffirent from `result_type` since branch
	/// to the loop header can't take any values.
	branch_type: Option<ValueType>,

	/// Stack height before entering in the block.
	start_height: usize,
}

#[derive(Clone)]
struct StackValue (ValueType, bool);

/// This is a compound stack that abstracts tracking height and weight of the value stack
/// and manipulation of the control stack.
struct Stack {
	values: Vec<StackValue>,
	control_stack: Vec<Frame>,
}

impl Stack {
	fn new() -> Stack {
		Stack { values: Vec::new(), control_stack: Vec::new() }
	}

	// fn new_from(stack: &Stack) -> Stack {
	// 	Stack { values: stack.values.clone(), control_stack: stack.control_stack.clone() }
	// } 

	/// Returns current weight of the value stack.
	fn weight(&self) -> u32 {
		self.values.iter().map(|v| value_cost(v.0)).sum()
	}

	/// Returns current height of the value stack.
	fn height(&self) -> usize {
		self.values.len()
	}

	/// Returns a reference to a frame by specified depth relative to the top of
	/// control stack.
	fn frame(&self, rel_depth: u32) -> Result<&Frame, &'static str> {
		let control_stack_height: usize = self.control_stack.len();
		let last_idx = control_stack_height.checked_sub(1).ok_or("control stack is empty")?;
		let idx = last_idx.checked_sub(rel_depth as usize).ok_or("control stack out-of-bounds")?;
		Ok(&self.control_stack[idx])
	}

	/// Mark successive instructions as unreachable.
	///
	/// This effectively makes stack polymorphic.
	fn mark_unreachable(&mut self) -> Result<(), &'static str> {
		let top_frame = self.control_stack.last_mut().ok_or("stack must be non-empty")?;
		top_frame.is_polymorphic = true;
		top_frame.unreachable_depth = 1;
		Ok(())
	}

	fn push_unreachable(&mut self) -> Result<(), &'static str> {
		let top_frame = self.control_stack.last_mut().ok_or("stack must be non-empty")?;
		top_frame.unreachable_depth += 1;
		Ok(())
	}

	fn pop_unreachable(&mut self) -> Result<u32, &'static str> {
		let top_frame = self.control_stack.last_mut().ok_or("stack must be non-empty")?;
		top_frame.unreachable_depth -= 1;
		Ok(top_frame.unreachable_depth)
	}

	/// Push control frame into the control stack.
	fn push_frame(&mut self, frame: Frame) {
		trace!("  Push control frame {:?}", frame);
		self.control_stack.push(frame);
	}

	/// Pop control frame from the control stack.
	///
	/// Returns `Err` if the control stack is empty.

	#[allow(clippy::let_and_return)]
	fn pop_frame(&mut self) -> Result<Frame, &'static str> {
		trace!("  Pop control frame");
		let frame = self.control_stack.pop().ok_or("stack must be non-empty");
		trace!("    {:?}", frame);
		frame
	}

	/// Truncate the height of value stack to the specified height.
	fn trunc(&mut self, new_height: usize) {
		trace!("  Truncate value stack to {}", new_height);
		self.values.truncate(new_height);
	}

	/// Push a value into the value stack.
	fn push_value(&mut self, value: StackValue) -> Result<(), &'static str> {
		trace!("  Push {:?} to value stack", value);
		self.values.push(value);
		if self.values.len() >= u32::MAX as usize {
			return Err("stack overflow")
		}
		Ok(())
	}

	/// Pop a value from the value stack.
	///
	/// Returns `Err` if the stack happen to be negative value after
	/// value popped.
	fn pop_value(&mut self) -> Result<Option<StackValue>, &'static str> {
		let top_frame = self.frame(0)?;
		if self.height() == top_frame.start_height {
			return if top_frame.is_polymorphic {
				Ok(None)
			} else {
				Err("trying to pop more values than pushed")
			}
		}

		if self.height() > 0 {
			let vt = self.values.pop();
			trace!("Pop {:?} from value stack", vt);
			Ok(vt)
		} else {
			Err("trying to pop more values than pushed")
		}
	}
}

fn value_cost(val: ValueType) -> u32 {
	match val {
		ValueType::I32 | ValueType::F32 => 4,
		ValueType::I64 | ValueType::F64 => 8,
	}
}

// struct FunctionContext {
// 	globals: Vec<ValueType>,
// 	locals: Vec<ValueType>,
// 	stack: Stack,
// 	result_type: Option<ValueType>
// }

/// This function expects the function to be validated.
pub fn compute(func_idx: u32, module: &elements::Module) -> Result<(u32, u32), &'static str> {
	use parity_wasm::elements::Instruction::*;

	let func_section = module.function_section().ok_or("No function section")?;
	let code_section = module.code_section().ok_or("No code section")?;
	let type_section = module.type_section().ok_or("No type section")?;

	// Get a signature and a body of the specified function.
	let func_sig_idx = func_section
		.entries()
		.get(func_idx as usize)
		.ok_or("Function is not found in func section")?
		.type_ref();
	let Type::Function(func_signature) = type_section
		.types()
		.get(func_sig_idx as usize)
		.ok_or("Function is not found in func section")?;
	let body = code_section
		.bodies()
		.get(func_idx as usize)
		.ok_or("Function body for the index isn't found")?;
	let instructions = body.code();

	// Get globals to resove their types
	let globals: Vec<ValueType> = if let Some(global_section) = module.global_section() {
		global_section
			.entries()
			.iter()
			.map(|g| g.global_type().content_type())
			.collect()
	} else {
		Vec::new()
	};

	let mut locals: Vec<StackValue> = func_signature.params().iter().map(|p| StackValue(*p, false)).collect();
	locals.extend(body.locals().iter().flat_map(|l| vec![StackValue(l.value_type(), true); l.count() as usize]));

	let mut stack = Stack::new();
	let mut max_weight: u32 = 0;
	let mut max_height: usize = 0;

	// Add implicit frame for the function. Breaks to this frame and execution of
	// the last end should deal with this frame.
	let func_results = func_signature.results();
	let param_weight: u32 = func_signature.params().iter().map(|v| value_cost(*v)).sum();

	let func_result_type = if func_results.is_empty() { None } else { Some(func_results[0]) };

	stack.push_frame(Frame {
		is_polymorphic: false,
		unreachable_depth: 0,
		result_type: func_result_type,
		branch_type: func_result_type,
		start_height: 0,
	});

	for opcode in instructions.elements() {
		let current_frame = stack.frame(0)?;
		if current_frame.is_polymorphic {
			match opcode {
				Block(ty) | Loop(ty) | If(ty) => {
					trace!("Entering unreachable block {:?}", opcode);
					stack.push_unreachable()?;
				},
				End => {
					let depth = stack.pop_unreachable()?;
					if depth == 0 {
						trace!("Exiting unreachable code");
						stack.pop_frame()?;
					} else {
						trace!("Exiting unreachable block");
					}
				},
				_ => {
					trace!("Skipping unreachable instruction {:?}", opcode);
				}
			}
			continue;
		}

		trace!("Processing opcode {:?}", opcode);

		match opcode {
			Nop => {},
			Block(ty) | Loop(ty) | If(ty) => {
				if let If(_) = *opcode {
					stack.pop_value()?;
				}
				let height = stack.height();
				let end_result = if let BlockType::Value(vt) = *ty { Some(vt) } else { None };
				stack.push_frame(Frame {
					is_polymorphic: false,
					unreachable_depth: 0,
					result_type: end_result,
					branch_type: if let Loop(_) = *opcode { None } else { end_result },
					start_height: height,
				});
			},
			Else => {
				// The frame at the top should be pushed by `If`. So we leave
				// it as is.
			},
			End => {
				// let frame = stack.pop_frame()?;
				// stack.trunc(frame.start_height);
				// if let Some(vt) = frame.result_type {
				// 	stack.push_value(vt)?;
				// }
				// // Push the frame back for now to allow for stack calculations. We'll get rid of it
				// // later
				// stack.push_frame(frame);
			},
			Unreachable => {
				stack.mark_unreachable()?;
			},
			Br(target) => {
				// Pop values for the destination block result.
				// if stack.frame(*target)?.branch_type.is_some() {
				// 	stack.pop_value()?;
				// }

				// This instruction unconditionally transfers control to the specified block,
				// thus all instruction until the end of the current block is deemed unreachable
				stack.mark_unreachable()?;
			},
			BrIf(target) => {
				// let target_type = stack.frame(*target)?.branch_type;
				// // Pop values for the destination block result.
				// if target_type.is_some() {
				// 	stack.pop_value()?;
				// }

				// Pop condition value.
				stack.pop_value()?;

				// Push values back.
				// if let Some(vt) = target_type {
				// 	stack.push_value(vt)?;
				// }
			},
			BrTable(br_table_data) => {
				// let default_type = stack.frame(br_table_data.default)?.branch_type;

				// Check that all jump targets have an equal number of parameters
				// for target in &*br_table_data.table {
				// 	if stack.frame(*target)?.branch_type != default_type {
				// 		return Err("Types of all jump-targets must be equal")
				// 	}
				// }

				// Because all jump targets have equal types, we can just take type of
				// the default branch.
				// if default_type.is_some() {
				// 	stack.pop_value()?;
				// }

				// This instruction doesn't let control flow to go further, since the control flow
				// should take either one of branches depending on the value or the default branch.
				stack.mark_unreachable()?;
			},
			Return => {
				// Pop return values of the function. Mark successive instructions as unreachable
				// since this instruction doesn't let control flow to go further.
				if func_result_type.is_some() {
					stack.pop_value()?;
				}
				stack.mark_unreachable()?;
			},
			Call(idx) => {
				let ty = resolve_func_type(*idx, module)?;

				// Pop values for arguments of the function.
				for _ in ty.params() {
					stack.pop_value()?;
				}

				// Push result of the function execution to the stack.
				let callee_results = ty.results();
				if !callee_results.is_empty() {
					stack.push_value(StackValue(callee_results[0], false))?;
				}
			},
			CallIndirect(x, _) => {
				let Type::Function(ty) =
					type_section.types().get(*x as usize).ok_or("Type not found")?;

				// Pop the offset into the function table.
				stack.pop_value()?;

				// Pop values for arguments of the function.
				for _ in ty.params() {
					stack.pop_value()?;
				}

				// Push result of the function execution to the stack.
				let callee_results = ty.results();
				if !callee_results.is_empty() {
					stack.push_value(StackValue(callee_results[0], false))?;
				}
			},
			Drop => {
				stack.pop_value()?;
			},
			Select => {
				// Pop two values and one condition.
				let val = stack.pop_value()?;
				stack.pop_value()?;
				stack.pop_value()?;

				// Push the selected value.
				if let Some(vt) = val {
					stack.push_value(vt)?;
				}
			},
			GetLocal(idx) => {
				let idx = *idx as usize;
				if idx >= locals.len() {
					return Err("Reference to a global is out of bounds")
				}
				stack.push_value(locals[idx])?;
			},
			SetLocal(_) => {
				stack.pop_value()?;
			},
			TeeLocal(idx) => {
				// This instruction pops and pushes the value, so
				// effectively it doesn't modify the stack height.
				let idx = *idx as usize;
				if idx >= locals.len() {
					return Err("Reference to a local is out of bounds")
				}
				stack.pop_value()?;
				stack.push_value(locals[idx])?;
			},
			GetGlobal(idx) => {
				let idx = *idx as usize;
				if idx >= globals.len() {
					return Err("Reference to a global is out of bounds")
				}
				stack.push_value(StackValue(globals[idx], false))?;
			},
			SetGlobal(_) => {
				stack.pop_value()?;
			},

			// These instructions pop the address and pushes the result
			I32Load(_, _) |
			I32Load8S(_, _) |
			I32Load8U(_, _) |
			I32Load16S(_, _) |
			I32Load16U(_, _) => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::I32, sv.1))?;
				}
			},
			I64Load(_, _) |
			I64Load8S(_, _) |
			I64Load8U(_, _) |
			I64Load16S(_, _) |
			I64Load16U(_, _) |
			I64Load32S(_, _) |
			I64Load32U(_, _) => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::I64, sv.1))?;
				}
			},
			F32Load(_, _) => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::F32, sv.1))?;
				}
			},
			F64Load(_, _) => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::F64, sv.1))?;
				}
			},

			I32Store(_, _) |
			I64Store(_, _) |
			F32Store(_, _) |
			F64Store(_, _) |
			I32Store8(_, _) |
			I32Store16(_, _) |
			I64Store8(_, _) |
			I64Store16(_, _) |
			I64Store32(_, _) => {
				// These instructions pop the address and the value.
				stack.pop_value()?;
				stack.pop_value()?;
			},

			CurrentMemory(_) => {
				// Pushes current memory size
				stack.push_value(StackValue(ValueType::I32, false))?;
			},
			GrowMemory(_) => {
				// Grow memory takes the value of pages to grow and pushes
				stack.pop_value()?;
				stack.push_value(StackValue(ValueType::I32, false))?;
			},

			I32Const(_) => {
				stack.push_value(StackValue(ValueType::I32, true))?;
			},
			I64Const(_) => {
				stack.push_value(StackValue(ValueType::I64, true))?;
			},
			F32Const(_) => {
				stack.push_value(StackValue(ValueType::F32, true))?;
			},
			F64Const(_) => {
				stack.push_value(StackValue(ValueType::F64, true))?;
			},

			I32Eqz | I64Eqz => {
				// These instructions pop the value and compare it against zero, and pushes
				// the result of the comparison.
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::I32, sv.1))?;
				}
			},

			I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS |
			I32GeU | I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU |
			I64GeS | I64GeU | F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge | F64Eq | F64Ne |
			F64Lt | F64Gt | F64Le | F64Ge => {
				// Comparison operations take two operands and produce one result.
				let Some(op1) = stack.pop_value()?;
				let Some(op2) = stack.pop_value()?;
				stack.push_value(StackValue(ValueType::I32, op1.1 && op2.1))?;
			},

			I32Clz | I32Ctz | I32Popcnt | I64Clz | I64Ctz | I64Popcnt | F32Abs | F32Neg |
			F32Ceil | F32Floor | F32Trunc | F32Nearest | F32Sqrt | F64Abs | F64Neg | F64Ceil |
			F64Floor | F64Trunc | F64Nearest | F64Sqrt => {
				// Unary operators take one operand and produce one result.
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(sv)?;
				}
			},

			I32Add | I32Sub | I32Mul | I32DivS | I32DivU | I32RemS | I32RemU | I32And | I32Or |
			I32Xor | I32Shl | I32ShrS | I32ShrU | I32Rotl | I32Rotr | I64Add | I64Sub |
			I64Mul | I64DivS | I64DivU | I64RemS | I64RemU | I64And | I64Or | I64Xor | I64Shl |
			I64ShrS | I64ShrU | I64Rotl | I64Rotr | F32Add | F32Sub | F32Mul | F32Div |
			F32Min | F32Max | F32Copysign | F64Add | F64Sub | F64Mul | F64Div | F64Min |
			F64Max | F64Copysign => {
				// Binary operators take two operands and produce one result.
				let Some(op1) = stack.pop_value()?;
				let Some(op2) = stack.pop_value()?;
				stack.push_value(StackValue(op1.0, op1.1 && op2.1))?;
			},

			// Conversion operators take one value and produce one result.
			I32WrapI64 | I32TruncSF32 | I32TruncUF32 | I32TruncSF64 | I32TruncUF64 |
			I32ReinterpretF32 => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::I32, sv.1))?;
				}
			},
			I64ExtendSI32 | I64ExtendUI32 | I64TruncSF32 | I64TruncUF32 | I64TruncSF64 |
			I64TruncUF64 | I64ReinterpretF64 => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::I64, sv.1))?;
				}
			},
			F32ConvertSI32 | F32ConvertUI32 | F32ConvertSI64 | F32ConvertUI64 | F32DemoteF64 |
			F32ReinterpretI32 => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::F32, sv.1))?;
				}
			},

			F64ConvertSI32 | F64ConvertUI32 | F64ConvertSI64 | F64ConvertUI64 | F64PromoteF32 |
			F64ReinterpretI64 => {
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(StackValue(ValueType::F64, sv.1))?;
				}
			},

			#[cfg(feature = "sign_ext")]
			SignExt(SignExtInstruction::I32Extend8S) |
			SignExt(SignExtInstruction::I32Extend16S) |
			SignExt(SignExtInstruction::I64Extend8S) |
			SignExt(SignExtInstruction::I64Extend16S) |
			SignExt(SignExtInstruction::I64Extend32S) =>
				if let Some(sv) = stack.pop_value()? {
					stack.push_value(sv)?;
				},
		}

		// If current value stack is heavier than maximal weight observed so far,
		// save the new weight.
		// However, we don't increase maximal value in unreachable code.
		if !stack.frame(0)?.is_polymorphic {
			let (cur_weight, cur_height) = (stack.weight(), stack.height());
			if cur_weight > max_weight {
				max_weight = cur_weight;
				trace!("Max weight is now {}", max_weight);
			}
			if cur_height > max_height {
				max_height = cur_height;
				trace!("Max height is now {}", max_height);
			}
		}

		// Post-execution stage: pop a control frame if block is ended
		if *opcode == End {
			stack.pop_frame()?;
		}
	}

	trace!("Final max stack height: {} + {}", ACTIVATION_FRAME_HEIGHT, max_height);
	trace!(
		"Final max stack weight: {} + {} + {}",
		ACTIVATION_FRAME_WEIGHT,
		max_weight,
		param_weight
	);

	Ok((
		ACTIVATION_FRAME_HEIGHT + max_height as u32,
		ACTIVATION_FRAME_WEIGHT + max_weight + param_weight,
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use parity_wasm::elements;

	#[cfg(feature = "trace-log")]
	use test_log::test;

	fn parse_wat(source: &str) -> elements::Module {
		elements::deserialize_buffer(&wat::parse_str(source).expect("Failed to wat2wasm"))
			.expect("Failed to deserialize the module")
	}

	#[test]
	fn simple_test() {
		let module = parse_wat(
			r#"
(module
	(func
		i32.const 1
			i32.const 2
				i32.const 3
				drop
			drop
		drop
	)
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 3, ACTIVATION_FRAME_WEIGHT + 12));
	}

	#[test]
	fn implicit_and_explicit_return() {
		let module = parse_wat(
			r#"
(module
	(func (result i32)
		i64.const 0
		return
	)
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 1, ACTIVATION_FRAME_WEIGHT + 8));
	}

	#[test]
	fn dont_count_in_unreachable() {
		let module = parse_wat(
			r#"
(module
  (memory 0)
  (func (result i32)
	unreachable
	grow_memory
  )
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT, ACTIVATION_FRAME_WEIGHT));
	}

	#[test]
	fn yet_another_test() {
		let module = parse_wat(
			r#"
(module
  (memory 0)
  (func
	;; Push two values and then pop them.
	;; This will make max depth to be equal to 2.
	i32.const 0
	i32.const 1
	drop
	drop

	;; Code after `unreachable` shouldn't have an effect
	;; on the max depth.
	unreachable
	i32.const 0
	i32.const 1
	i32.const 2
  )
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 2, ACTIVATION_FRAME_WEIGHT + 8));
	}

	#[test]
	fn call_indirect() {
		let module = parse_wat(
			r#"
(module
	(table $ptr 1 1 funcref)
	(elem $ptr (i32.const 0) func 1)
	(func $main
		(call_indirect (i32.const 0))
		(call_indirect (i32.const 0))
		(call_indirect (i32.const 0))
	)
	(func $callee
		i64.const 42
		drop
	)
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 1, ACTIVATION_FRAME_WEIGHT + 4));
	}

	#[test]
	fn breaks() {
		let module = parse_wat(
			r#"
(module
	(func $main
		block (result i32)
			block (result i32)
				i32.const 99
				br 1
			end
		end
		drop
	)
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 1, ACTIVATION_FRAME_WEIGHT + 4));
	}

	#[test]
	fn if_else_works() {
		let module = parse_wat(
			r#"
(module
	(func $main
		i32.const 7
		i32.const 1
		if (result i32)
			i32.const 42
		else
			i32.const 99
		end
		i32.const 97
		drop
		drop
		drop
	)
)
"#,
		);

		let res = compute(0, &module).unwrap();
		assert_eq!(res, (ACTIVATION_FRAME_HEIGHT + 3, ACTIVATION_FRAME_WEIGHT + 12));
	}
}
