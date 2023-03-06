use criterion::{
	criterion_group, criterion_main, measurement::Measurement, Bencher, BenchmarkGroup, Criterion,
};
use std::{
	fs::read,
	path::PathBuf,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use wasm_instrument::{
	gas_metering::{self, host_function, mutable_global, ConstantCostRules},
	parity_wasm::{deserialize_buffer, elements::Module, serialize},
};
use wasmi::{
	core::{Pages, TrapCode, F32},
	Caller, Config, Engine, Instance, Linker, Memory, StackLimits, Store, TypedFunc, Value,
};

/// Describes a gas metering strategy we want to benchmark.
///
/// Most strategies just need a subset of these functions. Hence we added default
/// implementations for all of them.
trait MeteringStrategy {
	/// The wasmi config we should be using for this strategy.
	fn config() -> Config {
		Config::default()
	}

	/// The strategy may or may not want to instrument the module.
	fn instrument_module(module: Module) -> Module {
		module
	}

	/// The strategy might need to define additional host functions.
	fn define_host_funcs(_linker: &mut Linker<u64>) {}

	/// The strategy might need to do some initialization of the wasm instance.
	fn init_instance(_module: &mut BenchInstance) {}
}

/// Don't do any metering at all. This is helpful as a baseline.
struct NoMetering;

/// Use wasmi's builtin fuel metering.
struct WasmiMetering;

/// Instrument the module using [`host_function::Injector`].
struct HostFunctionMetering;

/// Instrument the module using [`mutable_global::Injector`].
struct MutableGlobalMetering;

impl MeteringStrategy for NoMetering {}

impl MeteringStrategy for WasmiMetering {
	fn config() -> Config {
		let mut config = Config::default();
		config.consume_fuel(true);
		config
	}

	fn init_instance(module: &mut BenchInstance) {
		module.store.add_fuel(u64::MAX).unwrap();
	}
}

impl MeteringStrategy for HostFunctionMetering {
	fn instrument_module(module: Module) -> Module {
		let backend = host_function::Injector::new("env", "gas");
		gas_metering::inject(module, backend, &ConstantCostRules::default()).unwrap()
	}

	fn define_host_funcs(linker: &mut Linker<u64>) {
		// the instrumentation relies on the existing of this function
		linker
			.func_wrap("env", "gas", |mut caller: Caller<'_, u64>, amount_consumed: u64| {
				let gas_remaining = caller.data_mut();
				*gas_remaining =
					gas_remaining.checked_sub(amount_consumed).ok_or(TrapCode::OutOfFuel)?;
				Ok(())
			})
			.unwrap();
	}
}

impl MeteringStrategy for MutableGlobalMetering {
	fn instrument_module(module: Module) -> Module {
		let backend = mutable_global::Injector::new("gas_left");
		gas_metering::inject(module, backend, &ConstantCostRules::default()).unwrap()
	}

	fn init_instance(module: &mut BenchInstance) {
		// the instrumentation relies on the host to initialize the global with the gas limit
		// we just init to the maximum so it will never run out
		module
			.instance
			.get_global(&mut module.store, "gas_left")
			.unwrap()
			.set(&mut module.store, Value::I64(-1i64)) // the same as u64::MAX
			.unwrap();
	}
}

/// A wasm instance ready to be benchmarked.
struct BenchInstance {
	store: Store<u64>,
	instance: Instance,
}

impl BenchInstance {
	/// Create a new instance for the supplied metering strategy.
	///
	/// `wasm`: The raw wasm module for the benchmark.
	/// `define_host_func`: In here the caller can define additional host function.
	fn new<S, H>(wasm: &[u8], define_host_funcs: &H) -> Self
	where
		S: MeteringStrategy,
		H: Fn(&mut Linker<u64>),
	{
		let module = deserialize_buffer(wasm).unwrap();
		let instrumented_module = S::instrument_module(module);
		let input = serialize(instrumented_module).unwrap();
		let mut config = S::config();
		config.set_stack_limits(StackLimits::new(1024, 1024 * 1024, 64 * 1024).unwrap());
		let engine = Engine::new(&config);
		let module = wasmi::Module::new(&engine, &mut &input[..]).unwrap();
		let mut linker = Linker::new(&engine);
		S::define_host_funcs(&mut linker);
		define_host_funcs(&mut linker);
		// init host state with maximum gas_left (only used by host_function instrumentation)
		let mut store = Store::new(&engine, u64::MAX);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut bench_module = Self { store, instance };
		S::init_instance(&mut bench_module);
		bench_module
	}
}

/// Runs a benchmark for every strategy.
///
/// We require the closures to implement `Fn` as they are executed for every strategy and we
/// don't want them to change in between.
///
/// `group`: The benchmark group within the benchmarks will be executed.
/// `wasm`: The raw wasm module for the benchmark.
/// `define_host_func`: In here the caller can define additional host function.
/// `f`: In here the user should perform the benchmark. Will be executed for every strategy.
fn for_strategies<M, H, F>(group: &mut BenchmarkGroup<M>, wasm: &[u8], define_host_funcs: H, f: F)
where
	M: Measurement,
	H: Fn(&mut Linker<u64>),
	F: Fn(&mut Bencher<M>, &mut BenchInstance),
{
	let mut module = BenchInstance::new::<NoMetering, _>(wasm, &define_host_funcs);
	group.bench_function("no_metering", |bench| f(bench, &mut module));

	let mut module = BenchInstance::new::<WasmiMetering, _>(wasm, &define_host_funcs);
	group.bench_function("wasmi_builtin", |bench| f(bench, &mut module));

	let mut module = BenchInstance::new::<HostFunctionMetering, _>(wasm, &define_host_funcs);
	group.bench_function("host_function", |bench| f(bench, &mut module));

	let mut module = BenchInstance::new::<MutableGlobalMetering, _>(wasm, &define_host_funcs);
	group.bench_function("mutable_global", |bench| f(bench, &mut module));
}

/// Converts the `.wat` encoded `bytes` into `.wasm` encoded bytes.
fn wat2wasm(bytes: &[u8]) -> Vec<u8> {
	wat::parse_bytes(bytes).unwrap().into_owned()
}

fn fixture_dir() -> PathBuf {
	let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	path.push("benches");
	path.push("fixtures");
	path.push("wasm");
	path
}

fn gas_metered_coremark(c: &mut Criterion) {
	let mut group = c.benchmark_group("coremark");
	// Benchmark host_function::Injector
	let wasm_filename = "coremark_minimal.wasm";
	let bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	let define_host_funcs = |linker: &mut Linker<u64>| {
		linker
			.func_wrap("env", "clock_ms", || {
				SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
			})
			.unwrap();
	};
	for_strategies(&mut group, &bytes, define_host_funcs, |bench, module| {
		bench.iter(|| {
			let run = module.instance.get_typed_func::<(), F32>(&mut module.store, "run").unwrap();
			// Call the wasm!
			run.call(&mut module.store, ()).unwrap();
		})
	});
}

fn gas_metered_recursive_ok(c: &mut Criterion) {
	let mut group = c.benchmark_group("recursive_ok");
	const RECURSIVE_DEPTH: i32 = 8000;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/recursive_ok.wat"));
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let bench_call =
				module.instance.get_typed_func::<i32, i32>(&module.store, "call").unwrap();
			bench.iter(|| {
				let result = bench_call.call(&mut module.store, RECURSIVE_DEPTH).unwrap();
				assert_eq!(result, 0);
			})
		},
	);
}

fn gas_metered_fibonacci_recursive(c: &mut Criterion) {
	let mut group = c.benchmark_group("fibonacci_recursive");
	const FIBONACCI_REC_N: i64 = 10;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/fibonacci.wat"));
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let bench_call = module
				.instance
				.get_typed_func::<i64, i64>(&module.store, "fib_recursive")
				.unwrap();
			bench.iter(|| {
				bench_call.call(&mut module.store, FIBONACCI_REC_N).unwrap();
			});
		},
	);
}

fn gas_metered_fac_recursive(c: &mut Criterion) {
	let mut group = c.benchmark_group("factorial_recursive");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/factorial.wat"));
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let fac = module
				.instance
				.get_typed_func::<i64, i64>(&module.store, "recursive_factorial")
				.unwrap();
			bench.iter(|| {
				let result = fac.call(&mut module.store, 25).unwrap();
				assert_eq!(result, 7034535277573963776);
			})
		},
	);
}

fn gas_metered_count_until(c: &mut Criterion) {
	const COUNT_UNTIL: i32 = 100_000;
	let mut group = c.benchmark_group("count_until");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/count_until.wat"));
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let count_until = module
				.instance
				.get_typed_func::<i32, i32>(&module.store, "count_until")
				.unwrap();
			bench.iter(|| {
				let result = count_until.call(&mut module.store, COUNT_UNTIL).unwrap();
				assert_eq!(result, COUNT_UNTIL);
			})
		},
	);
}

fn gas_metered_vec_add(c: &mut Criterion) {
	fn test_for<A, B>(
		b: &mut Bencher,
		vec_add: TypedFunc<(i32, i32, i32, i32), ()>,
		mut store: &mut Store<u64>,
		mem: Memory,
		len: usize,
		vec_a: A,
		vec_b: B,
	) where
		A: IntoIterator<Item = i32>,
		B: IntoIterator<Item = i32>,
	{
		use core::mem::size_of;

		let ptr_result = 10;
		let len_result = len * size_of::<i64>();
		let ptr_a = ptr_result + len_result;
		let len_a = len * size_of::<i32>();
		let ptr_b = ptr_a + len_a;

		// Reset `result` buffer to zeros:
		mem.data_mut(&mut store)[ptr_result..ptr_result + (len * size_of::<i32>())].fill(0);
		// Initialize `a` buffer:
		for (n, a) in vec_a.into_iter().take(len).enumerate() {
			mem.write(&mut store, ptr_a + (n * size_of::<i32>()), &a.to_le_bytes()).unwrap();
		}
		// Initialize `b` buffer:
		for (n, b) in vec_b.into_iter().take(len).enumerate() {
			mem.write(&mut store, ptr_b + (n * size_of::<i32>()), &b.to_le_bytes()).unwrap();
		}

		// Prepare parameters and all Wasm `vec_add`:
		let params = (ptr_result as i32, ptr_a as i32, ptr_b as i32, len as i32);
		b.iter(|| {
			vec_add.call(&mut store, params).unwrap();
		});

		// Validate the result buffer:
		for n in 0..len {
			let mut buffer4 = [0x00; 4];
			let mut buffer8 = [0x00; 8];
			let a = {
				mem.read(&store, ptr_a + (n * size_of::<i32>()), &mut buffer4).unwrap();
				i32::from_le_bytes(buffer4)
			};
			let b = {
				mem.read(&store, ptr_b + (n * size_of::<i32>()), &mut buffer4).unwrap();
				i32::from_le_bytes(buffer4)
			};
			let actual_result = {
				mem.read(&store, ptr_result + (n * size_of::<i64>()), &mut buffer8).unwrap();
				i64::from_le_bytes(buffer8)
			};
			let expected_result = (a as i64) + (b as i64);
			assert_eq!(
				expected_result, actual_result,
				"given a = {a} and b = {b}, results diverge at index {n}"
			);
		}
	}

	let mut group = c.benchmark_group("memory_vec_add");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/memory-vec-add.wat"));
	const LEN: usize = 100_000;

	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let vec_add = module
				.instance
				.get_typed_func::<(i32, i32, i32, i32), ()>(&module.store, "vec_add")
				.unwrap();
			let mem = module.instance.get_memory(&module.store, "mem").unwrap();
			mem.grow(&mut module.store, Pages::new(25).unwrap()).unwrap();
			test_for(
				bench,
				vec_add,
				&mut module.store,
				mem,
				LEN,
				(0..LEN).map(|i| (i * i) as i32),
				(0..LEN).map(|i| (i * 10) as i32),
			)
		},
	);
}

fn gas_metered_tiny_keccak(c: &mut Criterion) {
	let mut group = c.benchmark_group("wasm_kernel::tiny_keccak");
	let wasm_filename = "wasm_kernel.wasm";
	let wasm_bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let prepare = module
				.instance
				.get_typed_func::<(), i32>(&module.store, "prepare_tiny_keccak")
				.unwrap();
			let keccak = module
				.instance
				.get_typed_func::<i32, ()>(&module.store, "bench_tiny_keccak")
				.unwrap();
			let test_data_ptr = prepare.call(&mut module.store, ()).unwrap();
			bench.iter(|| {
				keccak.call(&mut module.store, test_data_ptr).unwrap();
			})
		},
	);
}

fn gas_metered_global_bump(c: &mut Criterion) {
	const BUMP_AMOUNT: i32 = 100_000;
	let mut group = c.benchmark_group("global_bump");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/global_bump.wat"));
	for_strategies(
		&mut group,
		&wasm_bytes,
		|_| {},
		|bench, module| {
			let bump = module.instance.get_typed_func::<i32, i32>(&module.store, "bump").unwrap();
			bench.iter(|| {
				let result = bump.call(&mut module.store, BUMP_AMOUNT).unwrap();
				assert_eq!(result, BUMP_AMOUNT);
			})
		},
	);
}

criterion_group!(
	name = coremark;
	config = Criterion::default()
		.sample_size(10)
		.measurement_time(Duration::from_millis(275000))
		.warm_up_time(Duration::from_millis(1000));
	targets =
		 gas_metered_coremark,
);
criterion_group!(
	name = wasmi_fixtures;
	config = Criterion::default()
		.sample_size(10)
		.measurement_time(Duration::from_millis(250000))
	.warm_up_time(Duration::from_millis(1000));
	targets =
		gas_metered_recursive_ok,
		gas_metered_fibonacci_recursive,
		gas_metered_fac_recursive,
		gas_metered_count_until,
		gas_metered_vec_add,
		gas_metered_tiny_keccak,
		gas_metered_global_bump,
);
criterion_main!(coremark, wasmi_fixtures);
