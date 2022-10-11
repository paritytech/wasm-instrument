use crate::gas_metering::{add_grow_counter, inject_counter, inject_grow_counter, Backend, Rules};
use parity_wasm::{
	builder,
	elements::{self, IndexMap, Instruction, Module, ValueType},
};

/// Method 1. _(default, backwards-compatible)_ Inject invocations of gas charging
/// host function into each metering block. This is slow because calling imported functions is
/// a heavy operation.
///
/// `&str` value should contain the name of the module to import the `gas` function from.
pub struct ImportedFunctionInjector<'a>(pub &'a str);

impl Backend for ImportedFunctionInjector<'_> {
	fn inject<R: Rules>(&self, module: &Module, rules: &R) -> Result<Module, Module> {
		// Injecting gas counting external
		let mut mbuilder = builder::from_module(module.clone());
		let import_sig =
			mbuilder.push_signature(builder::signature().with_param(ValueType::I64).build_sig());
		//	       let gas_module_name = Self.gas_module;
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
