use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use wayexpand_backend_ibus::IbusEngineAdapter;
use wayexpand_core::{Config, ExpansionEngine, OrganizationPolicy};

fn large_import_config() -> Config {
    let mut source = String::with_capacity(1_500_000);
    for index in 0..10_000 {
        source.push_str(&format!(
            "[[expansion]]\ntrigger = ';wx{index:05}'\nreplacement = 'value {index}'\n"
        ));
    }
    Config::parse(&source).expect("10,000-entry imported-style configuration is valid")
}

fn ibus_engine_generation(c: &mut Criterion) {
    let config = large_import_config();
    let policy = OrganizationPolicy::default();
    let mut group = c.benchmark_group("ibus_10k_import_engine_generation");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(3));
    group.warm_up_time(std::time::Duration::from_secs(1));

    for contexts in [1usize, 4, 16] {
        group.bench_with_input(
            BenchmarkId::new("revalidate_for_each_context", contexts),
            &contexts,
            |b, &contexts| {
                b.iter(|| {
                    let mut adapters = Vec::with_capacity(contexts);
                    for _ in 0..contexts {
                        let engine = ExpansionEngine::new(config.clone()).unwrap();
                        adapters
                            .push(IbusEngineAdapter::with_policy(engine, policy.clone()).unwrap());
                    }
                    black_box(adapters);
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("share_validated_generation", contexts),
            &contexts,
            |b, &contexts| {
                b.iter(|| {
                    let mut effective = config.clone();
                    effective.apply_administrator_policy(&policy).unwrap();
                    let validated = Arc::new(effective.validate_snapshot().unwrap());
                    let mut adapters = Vec::with_capacity(contexts);
                    for _ in 0..contexts {
                        adapters.push(IbusEngineAdapter::with_validated_config(
                            Arc::clone(&validated),
                            policy.clone(),
                        ));
                    }
                    black_box(adapters);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, ibus_engine_generation);
criterion_main!(benches);
