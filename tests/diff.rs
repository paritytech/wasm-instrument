use std::{
	fs,
	io::{self, Read, Write},
	path::{Path, PathBuf},
};
use wasm_instrument::{self as instrument, gas_metering, parity_wasm::elements};
use wasmparser::validate;

fn slurp<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
	let mut f = fs::File::open(path)?;
	let mut buf = vec![];
	f.read_to_end(&mut buf)?;
	Ok(buf)
}

fn dump<P: AsRef<Path>>(path: P, buf: &[u8]) -> io::Result<()> {
	let mut f = fs::File::create(path)?;
	f.write_all(buf)?;
	Ok(())
}

fn run_diff_test<F: FnOnce(&[u8]) -> Vec<u8>>(
	test_dir: &str,
	in_name: &str,
	out_name: &str,
	test: F,
) {
	let mut fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	fixture_path.push("tests");
	fixture_path.push("fixtures");
	fixture_path.push(test_dir);
	fixture_path.push(in_name);

	let mut expected_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	expected_path.push("tests");
	expected_path.push("expectations");
	expected_path.push(test_dir);
	expected_path.push(out_name);

	let fixture_wasm = wat::parse_file(&fixture_path).expect("Failed to read fixture");
	validate(&fixture_wasm).expect("Fixture is invalid");

	let expected_wat = slurp(&expected_path).unwrap_or_default();
	let expected_wat = std::str::from_utf8(&expected_wat).expect("Failed to decode expected wat");

	let actual_wasm = test(fixture_wasm.as_ref());
	validate(&actual_wasm).expect("Result module is invalid");

	let actual_wat =
		wasmprinter::print_bytes(&actual_wasm).expect("Failed to convert result wasm to wat");

	if actual_wat != expected_wat {
		println!("difference!");
		println!("--- {}", expected_path.display());
		println!("+++ {} test {}", test_dir, out_name);
		for diff in diff::lines(expected_wat, &actual_wat) {
			match diff {
				diff::Result::Left(l) => println!("-{}", l),
				diff::Result::Both(l, _) => println!(" {}", l),
				diff::Result::Right(r) => println!("+{}", r),
			}
		}

		if std::env::var_os("BLESS").is_some() {
			dump(&expected_path, actual_wat.as_bytes()).expect("Failed to write to expected");
		} else {
			panic!();
		}
	}
}

mod stack_height {
	use super::*;

	macro_rules! def_stack_height_test {
		( $name:ident ) => {
			#[test]
			fn $name() {
				run_diff_test(
					"stack-height",
					concat!(stringify!($name), ".wat"),
					concat!(stringify!($name), ".wat"),
					|input| {
						let module =
							elements::deserialize_buffer(input).expect("Failed to deserialize");
						let instrumented = instrument::inject_stack_limiter(module, 1024)
							.expect("Failed to instrument with stack counter");
						elements::serialize(instrumented).expect("Failed to serialize")
					},
				);
			}
		};
	}

	def_stack_height_test!(simple);
	def_stack_height_test!(start);
	def_stack_height_test!(table);
	def_stack_height_test!(global);
	def_stack_height_test!(imports);
	def_stack_height_test!(many_locals);
	def_stack_height_test!(empty_functions);
}

mod gas {
	use super::*;

	macro_rules! def_gas_test {
		( ($input:ident, $name1:ident, $name2:ident) ) => {
			#[test]
			fn $name1() {
				run_diff_test(
					"gas",
					concat!(stringify!($input), ".wat"),
					concat!(stringify!($name1), ".wat"),
					|input| {
						let rules = gas_metering::ConstantCostRules::default();

						let module: elements::Module =
							elements::deserialize_buffer(input).expect("Failed to deserialize");
						let module = module.parse_names().expect("Failed to parse names");
						let backend = gas_metering::host_function::Injector::new("env", "gas");

						let instrumented = gas_metering::inject(module, backend, &rules)
							.expect("Failed to instrument with gas metering");
						elements::serialize(instrumented).expect("Failed to serialize")
					},
				);
			}

			#[test]
			fn $name2() {
				run_diff_test(
					"gas",
					concat!(stringify!($input), ".wat"),
					concat!(stringify!($name2), ".wat"),
					|input| {
						let rules = gas_metering::ConstantCostRules::default();

						let module: elements::Module =
							elements::deserialize_buffer(input).expect("Failed to deserialize");
						let module = module.parse_names().expect("Failed to parse names");
						let backend = gas_metering::mutable_global::Injector::new("gas_left");
						let instrumented = gas_metering::inject(module, backend, &rules)
							.expect("Failed to instrument with gas metering");
						elements::serialize(instrumented).expect("Failed to serialize")
					},
				);
			}
		};
	}

	def_gas_test!((ifs, ifs_host_fn, ifs_mut_global));
	def_gas_test!((simple, simple_host_fn, simple_mut_global));
	def_gas_test!((start, start_host_fn, start_mut_global));
	def_gas_test!((call, call_host_fn, call_mut_global));
	def_gas_test!((branch, branch_host_fn, branch_mut_global));
}
