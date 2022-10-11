use crate::gas_metering::{
	add_gas_counter, add_grow_counter, inject_counter, inject_grow_counter, Backend, Rules,
};
use parity_wasm::{
	builder,
	elements::{self, Instruction, Module, ValueType},
};

/// Injects a mutable global variable and a local function to the module to track
/// current gas left. The function is called in every metering block. In
/// case of falling out of gas, the global is set to the sentinel value `U64::MAX` and
/// `unreachable` instruction is called. The execution engine should take care of getting the
/// current global value and setting it back in order to sync the gas left value during an
/// execution.

pub struct MutableGlobalInjector<'a>(
	/// The export name of the gas tracking global.
	pub &'a str,
);

impl Backend for MutableGlobalInjector<'_> {
	/// Transforms a given module into one that tracks the gas charged during its execution.
	///
	/// The output module exports a mutable [i64] global with the specified name, which is used for
	/// tracking the gas left during an execution. Overall mechanics are similar to the
	/// [`ImportedFunctionInjector::inject()`][`super::ImportedFunctionInjector::inject`], aside
	/// from that a local injected gas counting function is called from each metering block intstead
	/// of an imported function, which should make the execution reasonably faster. Execution engine
	/// should take care of synchronizing the global with the runtime.
	fn inject<R: Rules>(&self, module: &Module, rules: &R) -> Result<Module, Module> {
		// Injecting the gas counting global
		let mut mbuilder = builder::from_module(module.clone());
		mbuilder.push_global(
			builder::global()
				.with_type(ValueType::I64)
				.mutable()
				.init_expr(Instruction::I64Const(0))
				.build(),
		);
		// Need to build the module to get the index of the added global
		let module = mbuilder.build();
		let gas_global_idx = (module.globals_space() as u32).saturating_sub(1);

		// Injecting the export entry for the gas counting global
		let mut mbuilder = builder::from_module(module);
		let ebuilder = builder::ExportBuilder::new();
		let global_export = ebuilder
			.field(self.0)
			.with_internal(elements::Internal::Global(gas_global_idx))
			.build();
		mbuilder.push_export(global_export);

		// Finally build the module
		let mut module = mbuilder.build();

		let gas_func_idx = module.functions_space() as u32;
		let mut need_grow_counter = false;
		let mut error = false;

		// Updating module sections.
		// - references to globals (all refs to global index >= 'gas_global_idx', should be
		//   incremented, because those are all non-imported ones)
		for section in module.sections_mut() {
			if let elements::Section::Code(code_section) = section {
				for func_body in code_section.bodies_mut() {
					if inject_counter(func_body.code_mut(), rules, gas_func_idx).is_err() {
						error = true;
						break
					}
					if rules.memory_grow_cost().enabled() &&
						inject_grow_counter(func_body.code_mut(), gas_func_idx + 1) > 0
					{
						need_grow_counter = true;
					}
				}
			}
		}

		if error {
			return Err(module)
		}

		let module = add_gas_counter(module, gas_global_idx);

		if need_grow_counter {
			Ok(add_grow_counter(module, rules, gas_func_idx))
		} else {
			Ok(module)
		}
	}
}
