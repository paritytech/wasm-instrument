use crate::gas_metering::{Backend, GasMeter};
use parity_wasm::elements::Module;

/// Injects invocations of the gas charging host function into each metering block.
///
/// This gas metering technique is slow because calling imported functions is a heavy operation. For
/// a faster gas metering see [`MutableGlobalInjector`][`super::MutableGlobalInjector`].
pub struct ImportedFunctionInjector {
	/// The name of the module to import the gas function from.
	module: &'static str,
	/// The name of the gas function to import.
	name: &'static str,
}

impl ImportedFunctionInjector {
	pub fn new(module: &'static str, name: &'static str) -> Self {
		Self { module, name }
	}
}

impl Backend for ImportedFunctionInjector {
	fn gas_meter(self, _module: Module) -> GasMeter {
		GasMeter::External { module: self.module, function: self.name }
	}
}
