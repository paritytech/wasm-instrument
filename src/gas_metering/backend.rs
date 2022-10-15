//! Provides backends for the gas metering instrumentation
use parity_wasm::{builder::FunctionDefinition, elements};

/// Implementation details of the specific method of the gas metering.
pub enum GasMeter {
	/// Gas metering with an external function.
	External {
		/// Name of the module to import the gas function from.
		module: &'static str,
		/// Name of the external gas function to be imported.
		function: &'static str,
	},
	/// Gas metering with a local function and a mutable global.
	Internal {
		/// Name of the mutable global to be exported.
		global: &'static str,
		/// Definition of the local gas counting function to be injected.
		function: FunctionDefinition,
	},
}

/// Under the hood part of the gas metering mechanics.
pub trait Backend {
	/// Provides the gas metering implementation details.  
	fn gas_meter(self, module: elements::Module) -> GasMeter;
}

/// Gas metering with an external function.
///
/// This is slow because calling imported functions is a heavy operation.
/// For a faster gas metering see [`super::mutable_global`].
pub mod host_function {
	use super::{Backend, GasMeter};
	use parity_wasm::elements::Module;
	/// Injects invocations of the gas charging host function into each metering block.
	pub struct Injector {
		/// The name of the module to import the gas function from.
		module: &'static str,
		/// The name of the gas function to import.
		name: &'static str,
	}

	impl Injector {
		pub fn new(module: &'static str, name: &'static str) -> Self {
			Self { module, name }
		}
	}

	impl Backend for Injector {
		fn gas_meter(self, _module: Module) -> GasMeter {
			GasMeter::External { module: self.module, function: self.name }
		}
	}
}

/// Gas metering with a mutable global.
pub mod mutable_global {
	use super::{Backend, GasMeter};
	use parity_wasm::{
		builder,
		elements::{self, Instruction, Module, ValueType},
	};
	/// Injects a mutable global variable and a local function to the module to track
	/// current gas left.
	///
	/// The function is called in every metering block. In case of falling out of gas, the global is
	/// set to the sentinel value `U64::MAX` and `unreachable` instruction is called. The execution
	/// engine should take care of getting the current global value and setting it back in order to
	/// sync the gas left value during an execution.
	pub struct Injector {
		/// The export name of the gas tracking global.
		pub global_name: &'static str,
	}

	impl Injector {
		pub fn new(global_name: &'static str) -> Self {
			Self { global_name }
		}
	}

	impl Backend for Injector {
		fn gas_meter(self, module: Module) -> GasMeter {
			// Build local gas function
			let fbuilder = builder::FunctionBuilder::new();
			let gas_func_sig =
				builder::SignatureBuilder::new().with_param(ValueType::I64).build_sig();
			let gas_global_idx = module.globals_space() as u32;
			let func = fbuilder
				.with_signature(gas_func_sig)
				.body()
				.with_instructions(elements::Instructions::new(vec![
					Instruction::GetGlobal(gas_global_idx),
					Instruction::GetLocal(0),
					Instruction::I64Sub,
					Instruction::TeeLocal(0),
					Instruction::I64Const(0),
					Instruction::I64LtS,
					Instruction::If(elements::BlockType::NoResult),
					Instruction::I64Const(-1i64), // sentinel val u64::MAX
					Instruction::SetGlobal(gas_global_idx),
					Instruction::Unreachable,
					Instruction::Else,
					Instruction::GetLocal(0),
					Instruction::SetGlobal(gas_global_idx),
					Instruction::End,
					Instruction::End,
				]))
				.build()
				.build();

			GasMeter::Internal { global: self.global_name, function: func }
		}
	}
}
