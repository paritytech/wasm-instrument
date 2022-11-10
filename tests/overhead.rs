use std::{
	fs::{read, read_dir},
	path::PathBuf,
};
use wasm_instrument::{
	gas_metering::{self, host_function, mutable_global, ConstantCostRules},
	inject_stack_limiter,
	parity_wasm::{deserialize_buffer, elements::Module, serialize},
};

fn fixture_dir() -> PathBuf {
	let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	path.push("benches");
	path.push("fixtures");
	path.push("wasm");
	path
}

fn overheads() {
	let mut results: Vec<_> = read_dir(fixture_dir())
		.unwrap()
		.map(|entry| {
			let entry = entry.unwrap();
			let (orig_len, orig_module) = {
				let bytes = read(&entry.path()).unwrap();
				let len = bytes.len();
				let module: Module = deserialize_buffer(&bytes).unwrap();
				(len, module)
			};
			let (gas_metered_host_fn_len, gas_metered_host_fn_module) = {
				let module = gas_metering::inject(
					orig_module.clone(),
					host_function::Injector::new("env", "gas"),
					&ConstantCostRules::default(),
				)
				.unwrap();
				let bytes = serialize(module.clone()).unwrap();
				let len = bytes.len();
				(len, module)
			};
			let (gas_metered_mut_global_len, gas_metered_mut_global_module) = {
				let module = gas_metering::inject(
					orig_module.clone(),
					mutable_global::Injector::new("gas_left"),
					&ConstantCostRules::default(),
				)
				.unwrap();
				let bytes = serialize(module.clone()).unwrap();
				let len = bytes.len();
				(len, module)
			};
			let stack_height_len = {
				let module = inject_stack_limiter(orig_module, 128).unwrap();
				let bytes = serialize(module).unwrap();
				bytes.len()
			};
			let gas_metered_host_fn_both_len = {
				let module = inject_stack_limiter(gas_metered_host_fn_module, 128).unwrap();
				let bytes = serialize(module).unwrap();
				bytes.len()
			};

			let gas_metered_mut_global_both_len = {
				let module = inject_stack_limiter(gas_metered_mut_global_module, 128).unwrap();
				let bytes = serialize(module).unwrap();
				bytes.len()
			};

			let overhead_host_fn = gas_metered_host_fn_both_len * 100 / orig_len;
			let overhead_mut_global = gas_metered_mut_global_both_len * 100 / orig_len;

			(
			    overhead_mut_global,
				format!(
					"{:30}: orig = {:4} kb, stack_limiter = {} %, gas_metered_host_fn =    {} %, both = {} %,\n \
					 {:69} gas_metered_mut_global = {} %, both = {} %",
					entry.file_name().to_str().unwrap(),
					orig_len / 1024,
					stack_height_len * 100 / orig_len,
					gas_metered_host_fn_len * 100 / orig_len,
				    overhead_host_fn,
				    "",
					gas_metered_mut_global_len * 100 / orig_len,
					overhead_mut_global,
				),
			)
		})
		.collect();
	results.sort_unstable_by(|a, b| b.0.cmp(&a.0));
	for entry in results {
		println!("{}", entry.1);
	}
}

/// Print the overhead of applying gas metering with host_function::Injector, stack
/// height limiting or both.
///
/// Use `cargo test print_size_overhead -- --nocapture`.
#[test]
fn print_size_overhead() {
	//    let mut_global_backend = mutable_global::Injector::new("gas_left");

	overheads();
	// overheads_for_backend::<wasm_instrument::gas_metering::host_function::Injector,
	// Clone>(host_fn_backend);
	// overheads_for_backend::<wasm_instrument::gas_metering::mutable_global::Injector,
	// Clone>(mut_global_backend);
}
