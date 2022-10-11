use crate::gas_metering::{add_grow_counter, inject_counter, inject_grow_counter, Backend, Rules};
use parity_wasm::{
	builder,
	elements::{self, IndexMap, Instruction, Module, ValueType},
};

/// Injects invocations of the gas charging host function into each metering block.
///
/// This gas metering technique is slow because calling imported functions is a heavy operation. For
/// a faster gas metering see [`MutableGlobalInjector`][`super::MutableGlobalInjector`].
pub struct ImportedFunctionInjector<'a>(
	/// The name of the module to import the `gas` function from.
	pub &'a str,
);

impl Backend for ImportedFunctionInjector<'_> {
	/// Transforms a given module into one that tracks the gas charged during its execution.
	///
	///
	/// The output module imports the `gas` function from the specified module with type signature
	/// [i64] -> []. The argument is the amount of gas required to continue execution. The external
	/// function is meant to keep track of the total amount of gas used and trap or otherwise halt
	/// execution of the runtime if the gas usage exceeds some allowed limit.
	///
	/// The body of each function is divided into metered blocks, and the calls to charge gas are
	/// inserted at the beginning of every such block of code. A metered block is defined so that,
	/// unless there is a trap, either all of the instructions are executed or none are. These are
	/// similar to basic blocks in a control flow graph, except that in some cases multiple basic
	/// blocks can be merged into a single metered block. This is the case if any path through the
	/// control flow graph containing one basic block also contains another.
	///
	/// Charging gas is at the beginning of each metered block ensures that 1) all instructions
	/// executed are already paid for, 2) instructions that will not be executed are not charged for
	/// unless execution traps, and 3) the number of calls to "gas" is minimized. The corollary is
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
	/// This routine runs in time linear in the size of the input module.
	///
	/// The function fails if the module contains any operation forbidden by gas rule set, returning
	/// the original module as an Err.
	fn inject<R: Rules>(&self, module: &Module, rules: &R) -> Result<Module, Module> {
		// Injecting gas counting external
		let mut mbuilder = builder::from_module(module.clone());
		let import_sig =
			mbuilder.push_signature(builder::signature().with_param(ValueType::I64).build_sig());
		mbuilder.push_import(
			builder::import()
				.module(self.0)
				.field("gas")
				.external()
				.func(import_sig)
				.build(),
		);

		// back to plain module
		let mut module = mbuilder.build();

		// calculate actual function index of the imported definition
		//    (subtract all imports that are NOT functions)
		let gas_func = module.import_count(elements::ImportCountType::Function) as u32 - 1;
		let total_func = module.functions_space() as u32;
		let mut need_grow_counter = false;
		let mut error = false;

		// Updating calling addresses (all calls to function index >= `gas_func` should be
		// incremented)
		for section in module.sections_mut() {
			match section {
				elements::Section::Code(code_section) =>
					for func_body in code_section.bodies_mut() {
						for instruction in func_body.code_mut().elements_mut().iter_mut() {
							if let Instruction::Call(call_index) = instruction {
								if *call_index >= gas_func {
									*call_index += 1
								}
							}
						}
						if inject_counter(func_body.code_mut(), rules, gas_func).is_err() {
							error = true;
							break
						}
						if rules.memory_grow_cost().enabled() &&
							inject_grow_counter(func_body.code_mut(), total_func) > 0
						{
							need_grow_counter = true;
						}
					},
				elements::Section::Export(export_section) => {
					for export in export_section.entries_mut() {
						if let elements::Internal::Function(func_index) = export.internal_mut() {
							if *func_index >= gas_func {
								*func_index += 1
							}
						}
					}
				},
				elements::Section::Element(elements_section) => {
					// Note that we do not need to check the element type referenced because in the
					// WebAssembly 1.0 spec, the only allowed element type is funcref.
					for segment in elements_section.entries_mut() {
						// update all indirect call addresses initial values
						for func_index in segment.members_mut() {
							if *func_index >= gas_func {
								*func_index += 1
							}
						}
					}
				},
				elements::Section::Start(start_idx) =>
					if *start_idx >= gas_func {
						*start_idx += 1
					},
				elements::Section::Name(s) =>
					for functions in s.functions_mut() {
						*functions.names_mut() =
							IndexMap::from_iter(functions.names().iter().map(|(mut idx, name)| {
								if idx >= gas_func {
									idx += 1;
								}

								(idx, name.clone())
							}));
					},
				_ => {},
			}
		}

		if error {
			return Err(module)
		}

		if need_grow_counter {
			Ok(add_grow_counter(module, rules, gas_func))
		} else {
			Ok(module)
		}
	}
}
