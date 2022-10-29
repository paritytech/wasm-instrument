use criterion::{
	criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
	Throughput,
};
use std::{
	fs::{read, read_dir},
	path::PathBuf,
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
		let bytes = read(&entry.path()).unwrap();
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

trait Prepare {
	// true => link gas host function
	// false => init gas_left global
	fn gas_preps(&self) -> bool;
}

impl Prepare for host_function::Injector {
	fn gas_preps(&self) -> bool {
		true
	}
}

impl Prepare for mutable_global::Injector {
	fn gas_preps(&self) -> bool {
		false
	}
}

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmi::{
	self,
	core::{Value, ValueType, F32},
	Caller, Config, Engine, Extern, Func, Instance, Linker, StackLimits, Store,
};

fn prepare_in_wasmi<P: Prepare + Backend>(backend: P, input: &[u8]) -> (Instance, Store<u64>) {
	let module = deserialize_buffer(input).unwrap();
	let gas_preps = backend.gas_preps();
	let instrumented_module =
		gas_metering::inject(module, backend, &ConstantCostRules::default()).unwrap();
	let input = serialize(instrumented_module).unwrap();
	// Prepare wasmi
	let engine = Engine::new(&bench_config());
	let module = wasmi::Module::new(&engine, &mut &input[..]).unwrap();
	// Host stores gas_left as `u64`
	type HostState = u64;
	// Init host state with maximum gas_left
	let mut store = Store::new(&engine, u64::MAX);
	// Create gas host function
	let host_gas = Func::wrap(&mut store, |mut caller: Caller<'_, HostState>, param: u64| {
		*caller.host_data_mut() -= param;
	});
	// Create clock_ms host function.
	// TBD: make it if needed: for coremark run
	// let host_clock_ms = Func::wrap(&mut store, || {
	// 	SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
	// });
	// // Link the host functions with the imported ones
	let mut linker = <Linker<HostState>>::new();
	// // Clock for time measuring from the coremark wasm
	// linker.define("env", "clock_ms", host_clock_ms).unwrap();
	// Link host gas function if needed
	if gas_preps {
		linker.define("env", "gas", host_gas).unwrap();
	}

	let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
	// Initialize gas_left global if needed
	if !gas_preps {
		instance
			.get_export(&mut store, "gas_left")
			.and_then(Extern::into_global)
			.unwrap()
			.set(&mut store, Value::I64(i64::MAX))
			.unwrap();
	};

	// TBD: make it if needed: for coremark run
	// TBD: convertion to TypedFunc<(), F32> from instance outside this func
	// let run = instance
	// 	.get_export(&mut store, "run")
	// 	.and_then(Extern::into_func)
	// 	.unwrap()
	// 	.typed::<(), F32>(&mut store)
	// 	.unwrap();

	(instance, store)
}

fn gas_metered_coremark(c: &mut Criterion) {
	let mut group = c.benchmark_group("Coremark, instrumented");
	// Benchmark host_function::Injector
	let wasm_filename = "coremark_minimal.wasm";
	let bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	let backend = host_function::Injector::new("env", "gas");
	let (instance, mut store) = prepare_in_wasmi(backend, &bytes);
	group.bench_function("with host_function::Injector", |bench| {
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

	// Benchmark mutable_global::Injector
	let backend = mutable_global::Injector::new("gas_left");
	let (instance, mut store) = prepare_in_wasmi(backend, &bytes);
	group.bench_function("with mutable_global::Injector", |bench| {
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

fn wasmi_execute_bare_call_16(c: &mut Criterion) {
	let mut group = c.benchmark_group("bare_call, instrumented");
	const REPETITIONS: usize = 20_000;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/bare_call.wat"));
	let params = &[
		Value::default(ValueType::I32),
		Value::default(ValueType::I64),
		Value::default(ValueType::F32),
		Value::default(ValueType::F64),
		Value::default(ValueType::I32),
		Value::default(ValueType::I64),
		Value::default(ValueType::F32),
		Value::default(ValueType::F64),
		Value::default(ValueType::I32),
		Value::default(ValueType::I64),
		Value::default(ValueType::F32),
		Value::default(ValueType::F64),
		Value::default(ValueType::I32),
		Value::default(ValueType::I64),
		Value::default(ValueType::F32),
		Value::default(ValueType::F64),
	];
	let results = &mut [Value::I32(0); 16];

	let backend = host_function::Injector::new("env", "gas");
	let (instance, mut store) = prepare_in_wasmi(backend, &wasm_bytes);
	let bare_call =
		instance.get_export(&store, "bare_call_16").and_then(Extern::into_func).unwrap();
	group.bench_function("with host_function::Injector", |bench| {
		bench.iter(|| {
			for _ in 0..REPETITIONS {
				bare_call.call(&mut store, params, results).unwrap();
			}
		})
	});

	let backend = mutable_global::Injector::new("gas_left");
	let (instance, mut store) = prepare_in_wasmi(backend, &wasm_bytes);
	let bare_call =
		instance.get_export(&store, "bare_call_16").and_then(Extern::into_func).unwrap();
	group.bench_function("with mutable_global::Injector", |bench| {
		bench.iter(|| {
			for _ in 0..REPETITIONS {
				bare_call.call(&mut store, params, results).unwrap();
			}
		})
	});
}

fn wasmi_execute_fibonacci_recursive(c: &mut Criterion) {
	let mut group = c.benchmark_group("fibonacci_recursive, instrumented");
	const FIBONACCI_REC_N: i64 = 10;
	let wasm_bytes = wat2wasm(include_bytes!("fixtures/wat/fibonacci.wat"));

	let backend = host_function::Injector::new("env", "gas");
	let (instance, mut store) = prepare_in_wasmi(backend, &wasm_bytes);
	let bench_call = instance
		.get_export(&store, "fib_recursive")
		.and_then(Extern::into_func)
		.unwrap();
	let mut result = [Value::I32(0)];

	group.bench_function("with host_function::Injector", |bench| {
		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I64(FIBONACCI_REC_N)], &mut result)
				.unwrap();
		});
	});

	let backend = mutable_global::Injector::new("gas_left");
	let (instance, mut store) = prepare_in_wasmi(backend, &wasm_bytes);

	let bench_call = instance
		.get_export(&store, "fib_recursive")
		.and_then(Extern::into_func)
		.unwrap();
	let mut result = [Value::I32(0)];

	group.bench_function("with mutable_global::Injector", |bench| {
		bench.iter(|| {
			bench_call
				.call(&mut store, &[Value::I64(FIBONACCI_REC_N)], &mut result)
				.unwrap();
		});
	});
}

criterion_group!(benches, gas_metering, stack_height_limiter);
criterion_group!(
	name = contest_backends;
	config = Criterion::default()
		.sample_size(10)
		.measurement_time(Duration::from_millis(250000))
		.warm_up_time(Duration::from_millis(1000));
	targets =
		 gas_metered_coremark,
);
criterion_group!(
	name = wasmi_execute;
	config = Criterion::default()
		.sample_size(10)
		.measurement_time(Duration::from_millis(250000));
//	.warm_up_time(Duration::from_millis(1000));
	targets =
//		 wasmi_execute_bare_call_16,
	wasmi_execute_fibonacci_recursive,
);
criterion_main!(benches, contest_backends, wasmi_execute);
