//! Cost of one GUI editing transaction on a large library: the editor clones
//! the whole configuration, edits the copy, and the save path validates it
//! before anything is written. This keeps the copy-then-validate model
//! honest at the documented 10,000-snippet maximum.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use wayexpand_core::Config;

fn library(count: usize) -> Config {
    let mut text = String::with_capacity(count * 160);
    for index in 0..count {
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":snippet{index:05}\"\n\
             replacement = \"Replacement text {index} with a sentence of ordinary length.\"\n\
             description = \"snippet {index}\"\ntags = [\"work\", \"email\"]\n\n"
        ));
    }
    Config::parse(&text).expect("benchmark configuration should validate")
}

fn bench_config_edit(c: &mut Criterion) {
    let mut clone_group = c.benchmark_group("config_edit_clone");
    for count in [100, 1_000, 10_000] {
        let config = library(count);
        clone_group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &config,
            |bench, config| {
                bench.iter(|| black_box(config.clone()));
            },
        );
    }
    clone_group.finish();

    let mut validate_group = c.benchmark_group("config_edit_validate");
    for count in [100, 1_000, 10_000] {
        let config = library(count);
        validate_group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &config,
            |bench, config| {
                bench.iter(|| black_box(config.validate().is_ok()));
            },
        );
    }
    validate_group.finish();
}

criterion_group!(benches, bench_config_edit);
criterion_main!(benches);
