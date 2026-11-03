use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use wayexpand_backend_evdev::benchmark_support::CaptureSource;

fn bench_capture_and_translate(c: &mut Criterion) {
    let mut source = CaptureSource::new();
    c.bench_function("evdev_capture_translate_ascii_pair", |bench| {
        bench.iter(|| black_box(source.capture_ascii_pair(30)))
    });
    let mut allocating_baseline = CaptureSource::new();
    c.bench_function(
        "evdev_capture_translate_ascii_pair_allocating_baseline",
        |bench| {
            bench.iter(|| black_box(allocating_baseline.capture_ascii_pair_allocating_baseline(30)))
        },
    );
}

criterion_group!(benches, bench_capture_and_translate);
criterion_main!(benches);
