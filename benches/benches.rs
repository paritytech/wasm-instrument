use criterion::{
	criterion_group, criterion_main, measurement::Measurement, Bencher, BenchmarkGroup, Criterion,
	Throughput,
};
use std::{
	fs::{read, read_dir},
	path::PathBuf,
	slice,
};
use wasm_instrument::{
	gas_metering::{self, host_function, mutable_global, Backend, ConstantCostRules},
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

fn any_fixture<F, M>(group: &mut BenchmarkGroup<M>, f: F)
where
	F: Fn(Module),
	M: Measurement,
{
	for entry in read_dir(fixture_dir()).unwrap() {
		let entry = entry.unwrap();
		let bytes = read(entry.path()).unwrap();
		group.throughput(Throughput::Bytes(bytes.len().try_into().unwrap()));
		group.bench_with_input(entry.file_name().to_str().unwrap(), &bytes, |bench, input| {
			bench.iter(|| f(deserialize_buffer(input).unwrap()))
		});
	}
}

fn gas_metering(c: &mut Criterion) {
	let mut group = c.benchmark_group("Gas Metering");
	any_fixture(&mut group, |module| {
		gas_metering::inject(
			module,
			host_function::Injector::new("env", "gas"),
			&ConstantCostRules::default(),
		)
		.unwrap();
	});
}

fn stack_height_limiter(c: &mut Criterion) {
	let mut group = c.benchmark_group("Stack Height Limiter");
	any_fixture(&mut group, |module| {
		inject_stack_limiter(module, 128).unwrap();
	});
}

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmi::{
	self,
	core::{Pages, Value, F32},
	Caller, Config, Engine, Extern, Func, Instance, Linker, Memory, StackLimits, Store,
};
fn prepare_module<P: Backend>(backend: P, input: &[u8]) -> (wasmi::Module, Store<u64>) {
	let module = deserialize_buffer(input).unwrap();
	let instrumented_module =
		gas_metering::inject(module, backend, &ConstantCostRules::default()).unwrap();
	let input = serialize(instrumented_module).unwrap();
	// Prepare wasmi
	let engine = Engine::new(&bench_config());
	let module = wasmi::Module::new(&engine, &mut &input[..]).unwrap();
	// Init host state with maximum gas_left
	let store = Store::new(&engine, u64::MAX);

	(module, store)
}

fn add_gas_host_func(linker: &mut Linker<u64>, store: &mut Store<u64>) {
	// Create gas host function
	let host_gas = Func::wrap(store, |mut caller: Caller<'_, u64>, param: u64| {
		*caller.host_data_mut() -= param;
	});
	// Link the gas host function
	linker.define("env", "gas", host_gas).unwrap();
}

fn add_gas_left_global(instance: &Instance, mut store: Store<u64>) -> Store<u64> {
	instance
		.get_export(&mut store, "gas_left")
		.and_then(Extern::into_global)
		.unwrap()
		.set(&mut store, Value::I64(-1i64)) // the same as u64::MAX
		.unwrap();
	store
}

fn gas_metered_coremark(c: &mut Criterion) {
	let mut group = c.benchmark_group("coremark, instrumented");
	// Benchmark host_function::Injector
	let wasm_filename = "coremark_minimal.wasm";
	let bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	group.bench_function("with host_function::Injector", |bench| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &bytes);
		// Link the host functions with the imported ones
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		// Create clock_ms host function.
		let host_clock_ms = Func::wrap(&mut store, || {
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
		});
		// Link the time measurer for the coremark wasm
		linker.define("env", "clock_ms", host_clock_ms).unwrap();

		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();

		bench.iter(|| {
			let run = instance
				.get_export(&mut store, "run")
				.and_then(Extern::into_func)
				.unwrap()
				.typed::<(), F32>(&mut store)
				.unwrap();
			// Call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});

	group.bench_function("with mutable_global::Injector", |bench| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &bytes);
		// Add the gas_left mutable global
		let mut linker = <Linker<u64>>::new();
		// Create clock_ms host function.
		let host_clock_ms = Func::wrap(&mut store, || {
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
		});
		// Link the time measurer for the coremark wasm
		linker.define("env", "clock_ms", host_clock_ms).unwrap();

		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);

		bench.iter(|| {
			let run = instance
				.get_export(&mut store, "run")
				.and_then(Extern::into_func)
				.unwrap()
				.typed::<(), F32>(&mut store)
				.unwrap();
			// Call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});
}

/// Converts the `.wat` encoded `bytes` into `.wasm` encoded bytes.
pub fn wat2wasm(bytes: &[u8]) -> Vec<u8> {
	wat::parse_bytes(bytes).unwrap().into_owned()
}

/// Returns a [`Config`] useful for benchmarking.
fn bench_config() -> Config {
	let mut config = Config::default();
	config.set_stack_limits(StackLimits::new(1024, 1024 * 1024, 64 * 1024).unwrap());
	config
}

fn gas_metered_recursive_ok(c: &mut Criterion) {
	let mut group = c.benchmark_group("recursive_ok, instrumented");
	const RECURSIVE_DEPTH: i32 = 8000;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/recursive_ok.wat"));

	group.bench_function("with host_function::Injector", |bench| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();

		let bench_call = instance.get_export(&store, "call").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I32(RECURSIVE_DEPTH)], &mut result)
				.unwrap();
			assert_eq!(result, [Value::I32(0)]);
		})
	});

	group.bench_function("with mutable_global::Injector", |bench| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);

		let bench_call = instance.get_export(&store, "call").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I32(RECURSIVE_DEPTH)], &mut result)
				.unwrap();
			assert_eq!(result, [Value::I32(0)]);
		})
	});
}

fn gas_metered_fibonacci_recursive(c: &mut Criterion) {
	let mut group = c.benchmark_group("fibonacci_recursive, instrumented");
	const FIBONACCI_REC_N: i64 = 10;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/fibonacci.wat"));

	group.bench_function("with host_function::Injector", |bench| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();

		let bench_call = instance
			.get_export(&store, "fib_recursive")
			.and_then(Extern::into_func)
			.unwrap();
		let mut result = [Value::I32(0)];

		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I64(FIBONACCI_REC_N)], &mut result)
				.unwrap();
		});
	});

	group.bench_function("with mutable_global::Injector", |bench| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);

		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);

		let bench_call = instance
			.get_export(&store, "fib_recursive")
			.and_then(Extern::into_func)
			.unwrap();
		let mut result = [Value::I32(0)];

		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I64(FIBONACCI_REC_N)], &mut result)
				.unwrap();
		});
	});
}

fn gas_metered_fac_recursive(c: &mut Criterion) {
	let mut group = c.benchmark_group("factorial_recursive, instrumented");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/factorial.wat"));

	group.bench_function("with host_function::Injector", |b| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let fac = instance
			.get_export(&store, "recursive_factorial")
			.and_then(Extern::into_func)
			.unwrap();
		let mut result = [Value::I64(0)];

		b.iter(|| {
			fac.call(&mut store, &[Value::I64(25)], &mut result).unwrap();
			assert_eq!(result, [Value::I64(7034535277573963776)]);
		})
	});

	group.bench_function("with mutable_global::Injector", |b| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);

		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);
		let fac = instance
			.get_export(&store, "recursive_factorial")
			.and_then(Extern::into_func)
			.unwrap();
		let mut result = [Value::I64(0)];

		b.iter(|| {
			fac.call(&mut store, &[Value::I64(25)], &mut result).unwrap();
			assert_eq!(result, [Value::I64(7034535277573963776)]);
		})
	});
}

fn gas_metered_count_until(c: &mut Criterion) {
	const COUNT_UNTIL: i32 = 100_000;
	let mut group = c.benchmark_group("count_until, instrumented");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/count_until.wat"));

	group.bench_function("with host_function::Injector", |b| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let count_until =
			instance.get_export(&store, "count_until").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		b.iter(|| {
			count_until.call(&mut store, &[Value::I32(COUNT_UNTIL)], &mut result).unwrap();
			assert_eq!(result, [Value::I32(COUNT_UNTIL)]);
		})
	});

	group.bench_function("with mutable_global::Injector", |b| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);

		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);
		let count_until =
			instance.get_export(&store, "count_until").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		b.iter(|| {
			count_until.call(&mut store, &[Value::I32(COUNT_UNTIL)], &mut result).unwrap();
			assert_eq!(result, [Value::I32(COUNT_UNTIL)]);
		})
	});
}

fn gas_metered_vec_add(c: &mut Criterion) {
	fn test_for<A, B>(
		b: &mut Bencher,
		vec_add: Func,
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
		let params = [
			Value::I32(ptr_result as i32),
			Value::I32(ptr_a as i32),
			Value::I32(ptr_b as i32),
			Value::I32(len as i32),
		];
		b.iter(|| {
			vec_add.call(&mut store, &params, &mut []).unwrap();
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

	let mut group = c.benchmark_group("memory_vec_add, instrumented");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/memory-vec-add.wat"));
	const LEN: usize = 100_000;

	group.bench_function("with host_function::Injector", |b| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let vec_add = instance.get_export(&store, "vec_add").and_then(Extern::into_func).unwrap();
		let mem = instance.get_export(&store, "mem").and_then(Extern::into_memory).unwrap();
		mem.grow(&mut store, Pages::new(25).unwrap()).unwrap();
		test_for(
			b,
			vec_add,
			&mut store,
			mem,
			LEN,
			(0..LEN).map(|i| (i * i) as i32),
			(0..LEN).map(|i| (i * 10) as i32),
		)
	});

	group.bench_function("with mutable_global::Injector", |b| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);
		let vec_add = instance.get_export(&store, "vec_add").and_then(Extern::into_func).unwrap();
		let mem = instance.get_export(&store, "mem").and_then(Extern::into_memory).unwrap();
		mem.grow(&mut store, Pages::new(25).unwrap()).unwrap();
		test_for(
			b,
			vec_add,
			&mut store,
			mem,
			LEN,
			(0..LEN).map(|i| (i * i) as i32),
			(0..LEN).map(|i| (i * 10) as i32),
		)
	});
}

fn gas_metered_tiny_keccak(c: &mut Criterion) {
	let mut group = c.benchmark_group("wasm_kernel::tiny_keccak, instrumented");
	let wasm_filename = "wasm_kernel.wasm";
	let wasm_bytes = read(fixture_dir().join(wasm_filename)).unwrap();

	group.bench_function("with host_function::Injector", |b| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let prepare = instance
			.get_export(&store, "prepare_tiny_keccak")
			.and_then(Extern::into_func)
			.unwrap();
		let keccak = instance
			.get_export(&store, "bench_tiny_keccak")
			.and_then(Extern::into_func)
			.unwrap();
		let mut test_data_ptr = Value::I32(0);
		prepare.call(&mut store, &[], slice::from_mut(&mut test_data_ptr)).unwrap();
		b.iter(|| {
			keccak.call(&mut store, slice::from_ref(&test_data_ptr), &mut []).unwrap();
		})
	});

	group.bench_function("with mutable_global::Injector", |b| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);
		let prepare = instance
			.get_export(&store, "prepare_tiny_keccak")
			.and_then(Extern::into_func)
			.unwrap();
		let keccak = instance
			.get_export(&store, "bench_tiny_keccak")
			.and_then(Extern::into_func)
			.unwrap();
		let mut test_data_ptr = Value::I32(0);
		prepare.call(&mut store, &[], slice::from_mut(&mut test_data_ptr)).unwrap();
		b.iter(|| {
			keccak.call(&mut store, slice::from_ref(&test_data_ptr), &mut []).unwrap();
		})
	});
}

fn gas_metered_global_bump(c: &mut Criterion) {
	const BUMP_AMOUNT: i32 = 100_000;
	let mut group = c.benchmark_group("global_bump, instrumented");
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/global_bump.wat"));

	group.bench_function("with host_function::Injector", |b| {
		let backend = host_function::Injector::new("env", "gas");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Link the host function with the imported one
		let mut linker = <Linker<u64>>::new();
		add_gas_host_func(&mut linker, &mut store);
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let bump = instance.get_export(&store, "bump").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		b.iter(|| {
			bump.call(&mut store, &[Value::I32(BUMP_AMOUNT)], &mut result).unwrap();
			assert_eq!(result, [Value::I32(BUMP_AMOUNT)]);
		})
	});

	group.bench_function("with mutable_global::Injector", |b| {
		let backend = mutable_global::Injector::new("gas_left");
		let (module, mut store) = prepare_module(backend, &wasm_bytes);
		// Add the gas_left mutable global
		let linker = <Linker<u64>>::new();
		let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
		let mut store = add_gas_left_global(&instance, store);
		let bump = instance.get_export(&store, "bump").and_then(Extern::into_func).unwrap();
		let mut result = [Value::I32(0)];

		b.iter(|| {
			bump.call(&mut store, &[Value::I32(BUMP_AMOUNT)], &mut result).unwrap();
			assert_eq!(result, [Value::I32(BUMP_AMOUNT)]);
		})
	});
}

criterion_group!(benches, gas_metering, stack_height_limiter);
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
