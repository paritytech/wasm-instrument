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

use gas_metering::Backend;
fn gas_metered_mod_len<B: Backend>(orig_module: Module, backend: B) -> (Module, usize) {
	let module = gas_metering::inject(orig_module, backend, &ConstantCostRules::default()).unwrap();
	let bytes = serialize(module.clone()).unwrap();
	let len = bytes.len();
	(module, len)
}

fn stack_limited_mod_len(module: Module) -> (Module, usize) {
	let module = inject_stack_limiter(module, 128).unwrap();
	let bytes = serialize(module.clone()).unwrap();
	let len = bytes.len();
	(module, len)
}

fn size_overheads_all() {
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

			let (gm_host_fn_module, gm_host_fn_len) = gas_metered_mod_len(
				orig_module.clone(),
				host_function::Injector::new("env", "gas"),
			);

			let (gm_mut_global_module, gm_mut_global_len) =
				gas_metered_mod_len(orig_module.clone(), mutable_global::Injector::new("gas_left"));

			let stack_limited_len = stack_limited_mod_len(orig_module).1;

			let (_gm_hf_sl_mod, gm_hf_sl_len) = stack_limited_mod_len(gm_host_fn_module.clone());

			let (_gm_mg_sl_module, gm_mg_sl_len) =
				stack_limited_mod_len(gm_mut_global_module.clone());

			let overhead_host_fn = gm_hf_sl_len * 100 / orig_len;
			let overhead_mut_global = gm_mg_sl_len * 100 / orig_len;

			let fname = entry.file_name();

			(
				overhead_mut_global,
				format!(
					"{:30}: orig = {:4} kb, stack_limiter = {} %, gas_metered_host_fn =    {} %, both = {} %,\n \
					 {:69} gas_metered_mut_global = {} %, both = {} %",
					fname.to_str().unwrap(),
					orig_len / 1024,
					stack_limited_len * 100 / orig_len,
					gm_host_fn_len * 100 / orig_len,
					overhead_host_fn,
					"",
					gm_mut_global_len * 100 / orig_len,
					overhead_mut_global,
				),
				wasmprinter::print_bytes(&gm_host_fn_module.into_bytes().unwrap())
					.expect("Failed to convert result wasm to wat"),
				format!("{}", fname.to_str().unwrap()),
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
	size_overheads_all();
}
