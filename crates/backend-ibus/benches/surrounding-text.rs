use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use wayexpand_backend_ibus::{IbusEngineAdapter, IBUS_CAP_SURROUNDING_TEXT};
use wayexpand_core::{Config, ExpansionEngine};

fn ibus_adapter() -> IbusEngineAdapter {
    let config =
        Config::parse("[[expansion]]\ntrigger = ';bench'\nreplacement = 'result'\n").unwrap();
    IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap())
}

fn surrounding_text_updates(c: &mut Criterion) {
    let mut group = c.benchmark_group("ibus_surrounding_text_update");
    group.sample_size(30);
    for char_count in [128usize, 4_096, 65_536] {
        // Multibyte scalars exercise the same Unicode indexing contract IBus
        // uses while keeping the benchmark content non-sensitive and static.
        let text = "界🙂é".repeat(char_count / 3);
        let scalar_count = text.chars().count() as u32;
        group.bench_with_input(
            BenchmarkId::from_parameter(scalar_count),
            &text,
            |b, text| {
                let mut adapter = ibus_adapter();
                adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
                b.iter(|| {
                    adapter.set_surrounding_text(black_box(text), scalar_count, scalar_count);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, surrounding_text_updates);
criterion_main!(benches);
