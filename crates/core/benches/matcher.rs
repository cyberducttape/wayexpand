use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
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

criterion_group!(benches, bench_matcher);
criterion_main!(benches);
