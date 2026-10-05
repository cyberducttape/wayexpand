use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use std::hint::black_box;
use wayexpand_core::{Config, ExpansionEngine, InputEvent};

fn config_with_expansions(count: usize) -> Config {
    let mut text = String::with_capacity(count * 64);
    for index in 0..count {
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":snippet{index:05}\"\nreplacement = \"value\"\n\n"
        ));
    }
    Config::parse(&text).expect("benchmark configuration should validate")
}

fn adversarial_config(count: usize) -> Config {
    let mut text = String::with_capacity(count * 768);
    for index in 0..count {
        // Exercise the expansion factors that the ordinary latency workload
        // intentionally avoids: aliases, case propagation, and NFD variants.
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":{index:05}é\"\naliases = ["
        ));
        // Keep the generated effective-trigger count within the production
        // cap at 10k snippets while still exercising a denser alias profile.
        let alias_count = if count <= 1_000 { 8 } else { 1 };
        for alias in 0..alias_count {
            if alias > 0 {
                text.push_str(", ");
            }
            text.push_str(&format!("\":a{index:05}_{alias}\""));
        }
        text.push_str("]\npropagate_case = true\nreplacement = \"value\"\n\n");
    }
    Config::parse(&text).expect("adversarial benchmark configuration should validate")
}

fn bench_matcher(c: &mut Criterion) {
    let mut group = c.benchmark_group("matcher_latency");

    for count in [100, 1_000, 10_000] {
        let mut engine = ExpansionEngine::new(config_with_expansions(count))
            .expect("benchmark engine should initialize");
        let trigger = format!(":snippet{:05}", count - 1);

        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &trigger,
            |bench, trigger| {
                bench.iter(|| {
                    for character in trigger.chars() {
                        black_box(engine.process(InputEvent::Text(character.to_string())));
                    }
                    black_box(engine.process(InputEvent::EndOfInput));
                });
            },
        );
    }

    group.finish();
}

fn bench_matcher_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("matcher_construction_amplified");

    for count in [1_000, 5_000, 10_000] {
        let config = adversarial_config(count);
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &config,
            |bench, config| {
                bench.iter_batched(
                    || config.clone(),
                    |config| {
                        black_box(
                            ExpansionEngine::new(config)
                                .expect("adversarial benchmark engine should initialize"),
                        );
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_matcher, bench_matcher_construction);
criterion_main!(benches);
