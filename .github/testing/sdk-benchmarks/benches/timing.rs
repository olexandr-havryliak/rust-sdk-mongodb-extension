use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sdk_benchmarks::*;
use std::{hint::black_box, time::Duration};

fn configuration() -> Criterion {
    let quick = std::env::var("BENCH_QUICK").as_deref() == Ok("1");
    let output = std::env::var("BENCH_CRITERION_DIR").unwrap_or_else(|_| "target/criterion".into());
    Criterion::default()
        .output_directory(std::path::Path::new(&output))
        .sample_size(if quick { 10 } else { 30 })
        .warm_up_time(Duration::from_millis(if quick { 20 } else { 500 }))
        .measurement_time(Duration::from_millis(if quick { 100 } else { 2000 }))
}

fn benchmarks(c: &mut Criterion) {
    initialize();
    let mut codec = c.benchmark_group("bson");
    for bytes in [256, 4096, 65536, 1048576] {
        codec.throughput(Throughput::Bytes(bytes as u64));
        codec.bench_with_input(BenchmarkId::new("roundtrip", bytes), &bytes, |b, &size| {
            b.iter(|| black_box(bson_roundtrip(black_box(size))))
        });
        codec.bench_with_input(
            BenchmarkId::new("owned_buffer", bytes),
            &bytes,
            |b, &size| b.iter(|| black_box(buffer_cycle(black_box(size)))),
        );
    }
    codec.finish();
    let mut abi = c.benchmark_group("abi_source_full_lifecycle");
    for (name, rows, bytes, metadata) in [
        ("short", 1, 256, false),
        ("cursor", 128, 4096, false),
        ("large", 8, 65536, false),
        ("1MiB", 1, 1048576, false),
        ("metadata", 128, 256, true),
        ("EOF", 0, 256, false),
    ] {
        abi.throughput(Throughput::Elements(rows.max(1) as u64));
        abi.bench_function(format!("sdk/{name}"), |b| {
            b.iter(|| black_box(sdk_source(rows, bytes, metadata)))
        });
        abi.bench_function(format!("raw/{name}"), |b| {
            b.iter(|| black_box(raw_source(rows, bytes, metadata)))
        });
    }
    abi.finish();
    c.bench_function("status/create_destroy", |b| b.iter(status_cycle));
    c.bench_function("metrics/host_owned_lifecycle", |b| {
        b.iter(|| black_box(metrics_roundtrip(128)))
    });
    c.bench_function("abi_source/error_cleanup", |b| {
        b.iter(|| {
            let mut cursor = SdkCursor::new(bson::doc! {"fail_at": 0});
            black_box(cursor.pull().unwrap_err());
        })
    });
}

criterion_group!(name = suite; config = configuration(); targets = benchmarks);
criterion_main!(suite);
