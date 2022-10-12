use criterion::{
	criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
	Throughput,
};
use std::{
	fs::{read, read_dir},
	path::PathBuf,
};
use wasm_instrument::{
	gas_metering::{Backend, ConstantCostRules, ImportedFunctionInjector, MutableGlobalInjector},
	inject_stack_limiter,
	parity_wasm::{deserialize_buffer, elements::Module, serialize},
};

fn fixture_dir() -> PathBuf {
	let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	path.push("benches");
	path.push("fixtures");
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
		ImportedFunctionInjector("env")
			.inject(&module, &ConstantCostRules::default())
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
	// true => gas host func needed,
	// false => global init gas_left
	fn gas_preps(&self) -> bool;
}

impl Prepare for ImportedFunctionInjector<'_> {
	fn gas_preps(&self) -> bool {
		true
	}
}

impl Prepare for MutableGlobalInjector<'_> {
	fn gas_preps(&self) -> bool {
		false
	}
}

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasmi::*;
fn prepare_in_wasmi<P: Prepare + Backend>(
	backend: P,
	input: &[u8],
) -> (TypedFunc<(), core::F32>, Store<u64>, Instance) {
	let module = deserialize_buffer(input).unwrap();

	let instrumented_module = backend.inject(&module, &ConstantCostRules::default()).unwrap();
	let input = serialize(instrumented_module).unwrap();
	// wasmi magic
	let engine = Engine::default();
	let module = wasmi::Module::new(&engine, &mut &input[..]).unwrap();

	// host stores gas_left as `u64`
	type HostState = u64;
	// we init host state with maximum gas_left
	let mut store = Store::new(&engine, u64::MAX);
	// create gas host function
	let host_gas = Func::wrap(&mut store, |mut caller: Caller<'_, HostState>, param: u64| {
		*caller.host_data_mut() -= param;
	});

	// create clock_ms host function
	let host_clock_ms = Func::wrap(&mut store, || {
		SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
	});

	// link the host functions with the imported ones
	let mut linker = <Linker<HostState>>::new();
	// clock for time measuring from the coremark wasm
	linker.define("env", "clock_ms", host_clock_ms).unwrap();
	// define host gas function if needed
	if backend.gas_preps() {
		linker.define("env", "gas", host_gas).unwrap();
	}

	let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();

	// set gas_left global if needed
	if !backend.gas_preps() {
		instance
			.get_export(&mut store, "gas_left")
			.and_then(Extern::into_global)
			.unwrap()
			.set(&mut store, core::Value::I64(i64::MAX))
			.unwrap();
	};

	let run = instance
		.get_export(&store, "run")
		.and_then(Extern::into_func)
		.unwrap()
		.typed::<(), core::F32, _>(&mut store)
		.unwrap();

	(run, store, instance)
}

fn gas_metered_coremark(c: &mut Criterion) {
	let mut group = c.benchmark_group("Coremark, instrumented");
	// Benchmark ImportedFunctionInjector
	let wasm_filename = "coremark_minimal.wasm";
	let bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	//	group.throughput(Throughput::Bytes(bytes.len().try_into().unwrap()));
	let (run, mut store, _instance) = prepare_in_wasmi(ImportedFunctionInjector("env"), &bytes);
	group.bench_function("with ImportedFunctionInjector", |bench| {
		bench.iter(|| {
			// call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});
	println!("gas spent: {}", u64::MAX - store.state());

	// Benchmark MutableGlobalInjector
	//	group.throughput(Throughput::Bytes(bytes.len().try_into().unwrap()));
	let (run, mut store, instance) = prepare_in_wasmi(MutableGlobalInjector("gas_left"), &bytes);
	group.bench_function("with MutableGlobalInjector", |bench| {
		bench.iter(|| {
			// call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});
	let gas_left = instance
		.get_export(&mut store, "gas_left")
		.and_then(Extern::into_global)
		.unwrap()
		.get(&mut store);
	if let core::Value::I64(gl) = gas_left {
		println!("gas left: {}", gl);
		println!("gas spent: {}", gl);
	};
}

criterion_group!(benches, gas_metering, stack_height_limiter);
criterion_group!(
	name = contest_injectors;
	config = Criterion::default()
		.sample_size(10)
		.measurement_time(Duration::from_millis(250000))
		.warm_up_time(Duration::from_millis(1000));
	targets =
		 gas_metered_coremark,
);
criterion_main!(benches, contest_injectors);
