use criterion::{
	criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
	Throughput,
};
use std::{
	fs::{read, read_dir},
	path::PathBuf,
};
use wasm_instrument::{
	gas_metering, inject_stack_limiter,
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
		group.bench_function(entry.file_name().to_str().unwrap(), |bench| {
			bench.iter(|| f(deserialize_buffer(&bytes).unwrap()))
		});
	}
}

pub fn gas_metering(c: &mut Criterion) {
	let mut group = c.benchmark_group("Gas Metering");
	any_fixture(&mut group, |module| {
		gas_metering::inject(module, &gas_metering::ConstantCostRules::default(), "env").unwrap();
	});
}

pub fn stack_height_limiter(c: &mut Criterion) {
	let mut group = c.benchmark_group("Stack Height Limiter");
	any_fixture(&mut group, |module| {
		inject_stack_limiter(module, 128).unwrap();
	});
}

criterion_group!(benches, gas_metering, stack_height_limiter);
criterion_main!(benches);
