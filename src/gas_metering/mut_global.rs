use crate::gas_metering::{Backend, GasMeter};
use parity_wasm::{
	builder,
	elements::{self, Instruction, Module, ValueType},
};

/// Injects a mutable global variable and a local function to the module to track
/// current gas left.
///
/// The function is called in every metering block. In case of falling out of gas, the global is set
/// to the sentinel value `U64::MAX` and `unreachable` instruction is called. The execution engine
/// should take care of getting the current global value and setting it back in order to sync the
/// gas left value during an execution.
pub struct MutableGlobalInjector {
	/// The export name of the gas tracking global.
	pub global_name: &'static str,
}

impl MutableGlobalInjector {
	pub fn new(global_name: &'static str) -> Self {
		Self { global_name }
	}
}

impl Backend for MutableGlobalInjector {
	fn gas_meter(self, module: Module) -> GasMeter {
		// Build local gas function
		let fbuilder = builder::FunctionBuilder::new();
		let gas_func_sig = builder::SignatureBuilder::new().with_param(ValueType::I64).build_sig();
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
