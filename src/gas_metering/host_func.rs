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
	gas_func_idx: u32,
}

impl ImportedFunctionInjector<'_> {
	pub fn new(module: &'static str) -> Self {
		Self { module, gas_func_idx: u32::MAX }
	}
}

impl Backend for ImportedFunctionInjector<'_> {
	/// TBD: update
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
		// back to plain module
		*module = mbuilder.build();
		// calculate actual function index of the imported definition
		//    (subtract all imports that are NOT functions)
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
