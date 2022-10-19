#[cfg(not(features = "std"))]
use alloc::collections::BTreeMap as Map;
use alloc::vec::Vec;
use parity_wasm::elements::{self, Internal};
#[cfg(features = "std")]
use std::collections::HashMap as Map;

use super::Context;

struct Thunk {
	/// The index of the signature in the type section.
	type_idx: u32,
	/// The number of parameters the function has.
	param_num: u32,
	// Index in function space of this thunk.
	idx: Option<u32>,
	callee_stack_cost: u32,
}

pub fn generate_thunks(
	ctx: &mut Context,
	mut module: elements::Module,
) -> Result<elements::Module, &'static str> {
	// First, we need to collect all function indices that should be replaced by thunks
	let mut replacement_map: Map<u32, Thunk> = {
		let types = module.type_section().map(|ts| ts.types()).unwrap_or(&[]);
		let exports = module.export_section().map(|es| es.entries()).unwrap_or(&[]);
		let elem_segments = module.elements_section().map(|es| es.entries()).unwrap_or(&[]);
		let start_func_idx = module.start_section();

		let exported_func_indices = exports.iter().filter_map(|entry| match entry.internal() {
			Internal::Function(function_idx) => Some(*function_idx),
			_ => None,
		});
		let table_func_indices =
			elem_segments.iter().flat_map(|segment| segment.members()).cloned();

		// Replacement map is at least export section size.
		let mut replacement_map: Map<u32, Thunk> = Map::new();

		for func_idx in exported_func_indices
			.chain(table_func_indices)
			.chain(start_func_idx.into_iter())
		{
			let callee_stack_cost = ctx.stack_cost(func_idx).ok_or("function index isn't found")?;

			// Don't generate a thunk if stack_cost of a callee is zero.
			if callee_stack_cost != 0 {
				let type_idx = ctx.func_type(func_idx).ok_or("type idx for thunk not found")?;
				let elements::Type::Function(func_ty) =
					types.get(type_idx as usize).ok_or("sig for thunk is not found")?;
				let param_num = func_ty.params().len() as u32;
				replacement_map
					.insert(func_idx, Thunk { type_idx, param_num, idx: None, callee_stack_cost });
			}
		}

		replacement_map
	};

	// Then, we generate a thunk for each original function.

	for (func_idx, thunk) in replacement_map.iter_mut() {
		let instrumented_call = instrument_call!(
			*func_idx,
			thunk.callee_stack_cost as i32,
			ctx.stack_height_global_idx(),
			ctx.stack_limit()
		);
		// Thunk body consist of:
		//  - argument pushing
		//  - instrumented call
		//  - end
		let mut thunk_body: Vec<elements::Instruction> =
			Vec::with_capacity(thunk.param_num as usize + instrumented_call.len() + 1);

		for arg_idx in 0..thunk.param_num {
			thunk_body.push(elements::Instruction::GetLocal(arg_idx));
		}
		thunk_body.extend_from_slice(&instrumented_call);
		thunk_body.push(elements::Instruction::End);

		let func_idx = insert_function(
			ctx,
			&mut module,
			thunk.type_idx,
			Vec::new(), // No declared local variables.
			elements::Instructions::new(thunk_body),
		)?;
		thunk.idx = Some(func_idx);
	}

	// And finally, fixup thunks in export and table sections.

	// Fixup original function index to a index of a thunk generated earlier.
	let fixup = |function_idx: &mut u32| {
		// Check whether this function is in replacement_map, since
		// we can skip thunk generation (e.g. if stack_cost of function is 0).
		if let Some(thunk) = replacement_map.get(function_idx) {
			*function_idx =
				thunk.idx.expect("At this point an index must be assigned to each thunk");
		}
	};

	for section in module.sections_mut() {
		match section {
			elements::Section::Export(export_section) => {
				for entry in export_section.entries_mut() {
					if let Internal::Function(function_idx) = entry.internal_mut() {
						fixup(function_idx)
					}
				}
			},
			elements::Section::Element(elem_section) => {
				for segment in elem_section.entries_mut() {
					for function_idx in segment.members_mut() {
						fixup(function_idx)
					}
				}
			},
			elements::Section::Start(start_idx) => fixup(start_idx),
			_ => {},
		}
	}

	Ok(module)
}

/// Inserts a new function into the module and returns it's index in the function space.
///
/// Specifically, inserts entires into the function section and the code section.
fn insert_function(
	ctx: &Context,
	module: &mut elements::Module,
	type_idx: u32,
	locals: Vec<elements::Local>,
	insns: elements::Instructions,
) -> Result<u32, &'static str> {
	let funcs = module
		.function_section_mut()
		.ok_or("insert function no function section")?
		.entries_mut();
	let new_func_idx = ctx
		.func_imports
		.checked_add(funcs.len() as u32)
		.ok_or("insert function func idx overflow")?;
	funcs.push(elements::Func::new(type_idx));

	let func_bodies =
		module.code_section_mut().ok_or("insert function no code section")?.bodies_mut();
	func_bodies.push(elements::FuncBody::new(locals, insns));

	Ok(new_func_idx)
}
