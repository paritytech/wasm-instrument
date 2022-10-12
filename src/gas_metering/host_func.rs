use crate::gas_metering::Backend;
use parity_wasm::{
	builder,
	elements::{self, Module, ValueType},
};

/// Injects invocations of the gas charging host function into each metering block.
///
/// This gas metering technique is slow because calling imported functions is a heavy operation. For
/// a faster gas metering see [`MutableGlobalInjector`][`super::MutableGlobalInjector`].
pub struct ImportedFunctionInjector<'a> {
	/// The name of the module to import the `gas` function from.
	pub module: &'a str,
	/// The index of the imported `gas` function.
	gas_func_idx: u32,
}

impl ImportedFunctionInjector<'_> {
	pub fn new(module: &'static str) -> Self {
		Self { module, gas_func_idx: u32::MAX }
	}
}

impl Backend for ImportedFunctionInjector<'_> {
	fn prepare(&mut self, module: &mut Module) -> (u32, u32) {
		// Injecting gas counting external
		let mut mbuilder = builder::from_module(module.clone());
		let import_sig =
			mbuilder.push_signature(builder::signature().with_param(ValueType::I64).build_sig());
		mbuilder.push_import(
			builder::import()
				.module(self.module)
				.field("gas")
				.external()
				.func(import_sig)
				.build(),
		);
		// Back to plain module
		*module = mbuilder.build();
		// Calculate actual function index of the imported definition
		// (subtract all imports that are NOT functions)
		self.gas_func_idx = module.import_count(elements::ImportCountType::Function) as u32 - 1;
		let total_func = module.functions_space() as u32;

		(self.gas_func_idx, total_func)
	}

	fn external_gas_func(&self) -> Option<u32> {
		Some(self.gas_func_idx)
	}

	fn local_gas_func(&self) -> Option<builder::FunctionDefinition> {
		None
	}
}
