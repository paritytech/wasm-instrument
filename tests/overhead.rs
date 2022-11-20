use std::{
	fs::{read, read_dir, ReadDir},
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

struct InstrumentedWasmResults {
	filename: String,
	original_module_len: usize,
	stack_limited_len: usize,
	gas_metered_host_fn_len: usize,
	gas_metered_mut_glob_len: usize,
	gas_metered_host_fn_then_stack_limited_len: usize,
	gas_metered_mut_glob_then_stack_limited_len: usize,
}

fn size_overheads_all(files: ReadDir) -> Vec<InstrumentedWasmResults> {
	files
		.map(|entry| {
			let entry = entry.unwrap();
			let filename = entry.file_name().into_string().unwrap();

			let (original_module_len, orig_module) = {
				let bytes = match entry.path().extension().unwrap().to_str() {
					Some("wasm") => read(&entry.path()).unwrap(),
					Some("wat") =>
						wat::parse_bytes(&read(&entry.path()).unwrap()).unwrap().into_owned(),
					_ => panic!("expected fixture_dir containing .wasm or .wat files only"),
				};

				let len = bytes.len();
				let module: Module = deserialize_buffer(&bytes).unwrap();
				(len, module)
			};

			let (gm_host_fn_module, gas_metered_host_fn_len) = gas_metered_mod_len(
				orig_module.clone(),
				host_function::Injector::new("env", "gas"),
			);

			let (gm_mut_global_module, gas_metered_mut_glob_len) =
				gas_metered_mod_len(orig_module.clone(), mutable_global::Injector::new("gas_left"));

			let stack_limited_len = stack_limited_mod_len(orig_module).1;

			let (_gm_hf_sl_mod, gas_metered_host_fn_then_stack_limited_len) =
				stack_limited_mod_len(gm_host_fn_module);

			let (_gm_mg_sl_module, gas_metered_mut_glob_then_stack_limited_len) =
				stack_limited_mod_len(gm_mut_global_module);

			InstrumentedWasmResults {
				filename,
				original_module_len,
				stack_limited_len,
				gas_metered_host_fn_len,
				gas_metered_mut_glob_len,
				gas_metered_host_fn_then_stack_limited_len,
				gas_metered_mut_glob_then_stack_limited_len,
			}
		})
		.collect()
}

fn calc_size_overheads() -> Vec<InstrumentedWasmResults> {
	let mut wasm_path = fixture_dir();
	wasm_path.push("wasm");

	let mut wat_path = fixture_dir();
	wat_path.push("wat");

	let mut results = size_overheads_all(read_dir(wasm_path).unwrap());
	let results_wat = size_overheads_all(read_dir(wat_path).unwrap());

	results.extend(results_wat);

	results
}

/// Print the overhead of applying gas metering, stack
/// height limiting or both.
///
/// Use `cargo test print_size_overhead -- --nocapture`.
#[test]
fn print_size_overhead() {
	let mut results = calc_size_overheads();
	results.sort_unstable_by(|a, b| {
		b.gas_metered_mut_glob_then_stack_limited_len
			.cmp(&a.gas_metered_mut_glob_then_stack_limited_len)
	});

	for r in results {
		let filename = r.filename;
		let original_size = r.original_module_len / 1024;
		let stack_limit = r.stack_limited_len * 100 / r.original_module_len;
		let host_fn = r.gas_metered_host_fn_len * 100 / r.original_module_len;
		let mut_glob = r.gas_metered_mut_glob_len * 100 / r.original_module_len;
		let host_fn_sl = r.gas_metered_host_fn_then_stack_limited_len * 100 / r.original_module_len;
		let mut_glob_sl =
			r.gas_metered_mut_glob_then_stack_limited_len * 100 / r.original_module_len;

		println!(
			"{filename:30}: orig = {original_size:4} kb, stack_limiter = {stack_limit} %, \
			  gas_metered_host_fn =    {host_fn} %, both = {host_fn_sl} %,\n \
			 {:69} gas_metered_mut_global = {mut_glob} %, both = {mut_glob_sl} %",
			""
		);
	}
}

/// Compare module size overhead of applying gas metering with two methods.
///
/// Use `cargo test print_gas_metered_sizes -- --nocapture`.
#[test]
fn print_gas_metered_sizes() {
	let overheads = calc_size_overheads();
	let mut results = overheads
		.iter()
		.map(|r| {
			let diff = (r.gas_metered_mut_glob_len * 100 / r.gas_metered_host_fn_len) as i32 - 100;
			(diff, r)
		})
		.collect::<Vec<(i32, &InstrumentedWasmResults)>>();
	results.sort_unstable_by(|a, b| b.0.cmp(&a.0));

	println!(
		"| {:28} | {:^16} | gas metered/host fn | gas metered/mut global | size diff |",
		"fixture", "original size",
	);
	println!("|{:-^30}|{:-^18}|{:-^21}|{:-^24}|{:-^11}|", "", "", "", "", "",);
	for r in results {
		let filename = &r.1.filename;
		let original_size = &r.1.original_module_len / 1024;
		let host_fn = &r.1.gas_metered_host_fn_len / 1024;
		let mut_glob = &r.1.gas_metered_mut_glob_len / 1024;
		let host_fn_percent = &r.1.gas_metered_host_fn_len * 100 / r.1.original_module_len;
		let mut_glob_percent = &r.1.gas_metered_mut_glob_len * 100 / r.1.original_module_len;
		let host_fn = format!("{host_fn} kb ({host_fn_percent:}%)");
		let mut_glob = format!("{mut_glob} kb ({mut_glob_percent:}%)");
		let diff = &r.0;
		println!(
			"| {filename:28} | {original_size:13} kb | {host_fn:>19} | {mut_glob:>22} | {diff:+8}% |"
		);
	}
}
