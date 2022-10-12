use crate::gas_metering::Backend;
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
pub struct MutableGlobalInjector<'a> {
	/// The export name of the gas tracking global.
	pub global_name: &'a str,
	/// The index of the `gas_left` global.
	gas_global_idx: u32,
}

impl MutableGlobalInjector<'_> {
	pub fn new(global_name: &'static str) -> Self {
		Self { global_name, gas_global_idx: u32::MAX }
	}
}

impl Backend for MutableGlobalInjector<'_> {
	fn prepare(&mut self, module: &mut Module) -> (u32, u32) {
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
		let temp_module = mbuilder.build();
		self.gas_global_idx = (temp_module.globals_space() as u32).saturating_sub(1);

		// Injecting the export entry for the gas counting global
		let mut mbuilder = builder::from_module(temp_module);
		let ebuilder = builder::ExportBuilder::new();
		let global_export = ebuilder
			.field(self.global_name)
			.with_internal(elements::Internal::Global(self.gas_global_idx))
			.build();
		mbuilder.push_export(global_export);

		// Finally build the module
		*module = mbuilder.build();

		// Local gas_func is added later and gets this idx
		let gas_func_idx = module.functions_space() as u32;
		let total_funcs = gas_func_idx + 1;

		(gas_func_idx, total_funcs)
	}

	fn external_gas_func(&self) -> Option<u32> {
		None
	}

	fn local_gas_func(&self) -> Option<builder::FunctionDefinition> {
		let fbuilder = builder::FunctionBuilder::new();
		let gas_func_sig = builder::SignatureBuilder::new().with_param(ValueType::I64).build_sig();
		let func = fbuilder
			.with_signature(gas_func_sig)
			.body()
			.with_instructions(elements::Instructions::new(vec![
				Instruction::GetGlobal(self.gas_global_idx),
				Instruction::GetLocal(0),
				Instruction::I64Sub,
				Instruction::TeeLocal(0),
				Instruction::I64Const(0),
				Instruction::I64LtS,
				Instruction::If(elements::BlockType::NoResult),
				Instruction::I64Const(-1i64), // sentinel val u64::MAX
				Instruction::SetGlobal(self.gas_global_idx),
				Instruction::Unreachable,
				Instruction::Else,
				Instruction::GetLocal(0),
				Instruction::SetGlobal(self.gas_global_idx),
				Instruction::End,
				Instruction::End,
			]))
			.build()
			.build();

		Some(func)
	}
}
