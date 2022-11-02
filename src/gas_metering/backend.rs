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

use super::Rules;
/// Under the hood part of the gas metering mechanics.
pub trait Backend {
	/// Provides the gas metering implementation details.  
	fn gas_meter<R: Rules>(self, module: &elements::Module, rules: &R) -> GasMeter;
}

/// Gas metering with an external function.
///
/// This is slow because calling imported functions is a heavy operation.
/// For a faster gas metering see [`super::mutable_global`].
pub mod host_function {
	use super::{Backend, GasMeter, Rules};
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
		fn gas_meter<R: Rules>(self, _module: &Module, _rules: &R) -> GasMeter {
			GasMeter::External { module: self.module, function: self.name }
		}
	}
}

/// Gas metering with a mutable global.
pub mod mutable_global {
	use super::{Backend, GasMeter, Rules};
	use alloc::vec;
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
		fn gas_meter<R: Rules>(self, module: &Module, rules: &R) -> GasMeter {
			// Build local gas function
			let fbuilder = builder::FunctionBuilder::new();
			let gas_func_sig =
				builder::SignatureBuilder::new().with_param(ValueType::I64).build_sig();
			let gas_global_idx = module.globals_space() as u32;

			let mut func_instructions = vec![
				Instruction::GetGlobal(gas_global_idx),
				// charging for this function execution itself
				Instruction::I64Const(0), // gas func overhead cost, the value is actualized below
				Instruction::I64Sub,
				Instruction::TeeLocal(1),
				Instruction::GetLocal(0),
				Instruction::I64GeU,
				Instruction::If(elements::BlockType::NoResult),
				Instruction::GetLocal(1),
				Instruction::GetLocal(0),
				Instruction::I64Sub,
				Instruction::SetGlobal(gas_global_idx),
				Instruction::Return,
				Instruction::End,
				// sentinel val u64::MAX
				Instruction::I64Const(-1i64),           // non-charged instruction
				Instruction::SetGlobal(gas_global_idx), // non-charged instruction
				Instruction::Unreachable,               // non-charged instruction
				Instruction::End,
			];

			// calculate gas used for the gas charging func execution itself
			let mut gas_fn_cost = func_instructions.iter().fold(0, |cost, instruction| {
				cost + (rules.instruction_cost(instruction).unwrap_or(0) as i64)
			});

			// don't charge for the instructions used to fail when out of gas
			let fail_cost = vec![
				Instruction::I64Const(-1i64),           // non-charged instruction
				Instruction::SetGlobal(gas_global_idx), // non-charged instruction
				Instruction::Unreachable,               // non-charged instruction
			]
			.iter()
			.fold(0, |cost, instruction| {
				cost + (rules.instruction_cost(instruction).unwrap_or(0) as i64)
			});

			gas_fn_cost -= fail_cost;

			// update the charged overhead cost
			func_instructions[1] = Instruction::I64Const(gas_fn_cost);

			let func = fbuilder
				.with_signature(gas_func_sig)
				.body()
				.with_locals([elements::Local::new(1, ValueType::I64)])
				.with_instructions(elements::Instructions::new(func_instructions))
				.build()
				.build();

			GasMeter::Internal { global: self.global_name, function: func }
		}
	}
}
