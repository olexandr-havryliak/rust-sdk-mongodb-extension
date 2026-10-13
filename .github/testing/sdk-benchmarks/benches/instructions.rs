use gungraun::{library_benchmark, library_benchmark_group, main, Dhat, LibraryBenchmarkConfig};
use std::hint::black_box;

fn prepare(case: (i64, usize, bool)) -> (i64, usize, bool) {
    sdk_benchmarks::initialize();
    case
}

#[library_benchmark(setup = prepare)]
#[bench::short((1, 256, false))]
#[bench::cursor((128, 4096, false))]
#[bench::large((8, 65536, false))]
#[bench::one_mib((1, 1048576, false))]
#[bench::metadata((128, 256, true))]
#[bench::eof((0, 256, false))]
fn sdk(case: (i64, usize, bool)) -> u64 {
    black_box(sdk_benchmarks::sdk_source(case.0, case.1, case.2))
}

#[library_benchmark(setup = prepare)]
#[bench::short((1, 256, false))]
#[bench::cursor((128, 4096, false))]
#[bench::large((8, 65536, false))]
#[bench::one_mib((1, 1048576, false))]
#[bench::metadata((128, 256, true))]
#[bench::eof((0, 256, false))]
fn raw(case: (i64, usize, bool)) -> u64 {
    black_box(sdk_benchmarks::raw_source(case.0, case.1, case.2))
}

#[library_benchmark]
#[bench::small(256)]
#[bench::medium(4096)]
#[bench::large(65536)]
#[bench::one_mib(1048576)]
fn codec(bytes: usize) -> usize {
    black_box(sdk_benchmarks::bson_roundtrip(bytes))
}

#[library_benchmark]
fn status() {
    sdk_benchmarks::status_cycle();
}

library_benchmark_group!(name = suite; benchmarks = sdk, raw, codec, status);
main!(config = LibraryBenchmarkConfig::default().tool(Dhat::default()); library_benchmark_groups = suite);
