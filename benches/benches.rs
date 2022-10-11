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
	parity_wasm::{deserialize_buffer, elements::Module},
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
	fn need_host_gas(&self) -> bool;
}

impl Prepare for ImportedFunctionInjector<'_> {
	fn need_host_gas(&self) -> bool {
		true
	}
}

impl Prepare for MutableGlobalInjector<'_> {
	fn need_host_gas(&self) -> bool {
		false
	}
}

use wasmi::*;
use wasmi_core::F32;
fn prepare_in_wasmi<P: Prepare + Backend>(
	backend: P,
	input: &[u8],
) -> (TypedFunc<(), F32>, Store<u64>) {
	use std::time::{SystemTime, UNIX_EPOCH};
	let module = deserialize_buffer(input).unwrap();

	let instrumented_module = backend.inject(&module, &ConstantCostRules::default());
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
	if backend.need_host_gas() {
		linker.define("env", "gas", host_gas).unwrap();
	};
	linker.define("env", "clock_ms", host_clock_ms).unwrap();
	let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();

	let run = instance
		.get_export(&store, "run")
		.and_then(Extern::into_func)
		.unwrap()
		.typed::<(), F32, _>(&mut store)
		.unwrap();

	(run, store)
}

fn gas_metered_coremark(c: &mut Criterion) {
	let mut group = c.benchmark_group("Coremark, instrumented");
	// Benchmark ImportedFunctionInjector
	let wasm_filename = "coremark_minimal.wasm";
	let bytes = read(fixture_dir().join(wasm_filename)).unwrap();
	//	group.throughput(Throughput::Bytes(bytes.len().try_into().unwrap()));
	let (run, mut store) = prepare_in_wasmi(ImportedFunctionInjector("env"), &bytes);
	group.bench_function("with ImportedFunctionInjector", |bench| {
		bench.iter(|| {
			// call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});

	// Benchmark MutableGlobalInjector
	//	group.throughput(Throughput::Bytes(bytes.len().try_into().unwrap()));
	let (run, mut store) = prepare_in_wasmi(MutableGlobalInjector("gas_left"), &bytes);
	group.bench_function("metered with MutableGlobalInjector", |bench| {
		bench.iter(|| {
			// call the wasm!
			run.call(&mut store, ()).unwrap();
		})
	});
}

//criterion_group!(benches, gas_metering, stack_height_limiter);
criterion_group!(benches, gas_metered_coremark);
criterion_main!(benches);
