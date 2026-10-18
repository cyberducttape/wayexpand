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
    Config::parse(&adversarial_config_text(count))
        .expect("adversarial benchmark configuration should validate")
}

fn adversarial_config_text(count: usize) -> String {
    let mut text = String::with_capacity(count * 768);
    for index in 0..count {
        // Exercise the expansion factors that the ordinary latency workload
        // intentionally avoids: aliases, case propagation, and NFD variants.
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":{index:05}é\"\naliases = ["
        ));
        // Exercise the largest alias profile that remains under the effective
        // trigger cap at the documented 10k-snippet scale. With this trigger,
        // case propagation and NFC/NFD variants each amplify aliases further.
        let alias_count = if count <= 1_000 { 8 } else { 2 };
        for alias in 0..alias_count {
            if alias > 0 {
                text.push_str(", ");
            }
            text.push_str(&format!("\":a{index:05}_{alias}\""));
        }
        text.push_str("]\npropagate_case = true\nreplacement = \"value\"\n\n");
    }
    text
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

fn bench_long_buffer_keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("long_buffer_keystroke");
    let config = Config::parse(
        "[settings]\nmax_buffer_chars = 4096\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"value\"",
    )
    .expect("benchmark configuration should validate");
    // Plain ASCII prose, and text with decomposed accents that is not NFC.
    for (name, unit) in [("ascii", "lorem ipsum "), ("nfd", "cafe\u{301} ")] {
        let mut engine =
            ExpansionEngine::new(config.clone()).expect("benchmark engine should initialize");
        let filler = unit.repeat(4096 / unit.chars().count() + 1);
        engine.process(InputEvent::Text(filler));
        group.bench_function(BenchmarkId::new("type_and_backspace", name), |bench| {
            bench.iter(|| {
                black_box(engine.process(InputEvent::Text("a".into())));
                black_box(engine.process(InputEvent::Backspace));
            });
        });
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

/// Buffers built to be expensive for grapheme-aware editing and
/// normalization rather than typical prose: very long combining sequences
/// (one grapheme cluster that spans the whole buffer), ZWJ emoji sequences,
/// regional-indicator pairs, and Hangul jamo that compose under NFC.
fn bench_unicode_worst_case(c: &mut Criterion) {
    let mut group = c.benchmark_group("unicode_worst_case_keystroke");
    let config = Config::parse(
        "[settings]\nmax_buffer_chars = 4096\n[[expansion]]\ntrigger = \":café\"\npropagate_case = true\nreplacement = \"value\"",
    )
    .expect("benchmark configuration should validate");
    let one_cluster = format!("e{}", "\u{301}".repeat(4095));
    let zalgo = format!("a{} ", "\u{301}\u{316}\u{35c}\u{489}".repeat(8));
    let cases: [(&str, String); 5] = [
        ("single_cluster", one_cluster),
        ("zalgo", zalgo.repeat(4096 / zalgo.chars().count() + 1)),
        ("zwj_emoji", "👩\u{200d}👩\u{200d}👧\u{200d}👦 ".repeat(512)),
        ("regional_indicators", "🇩🇪🇮🇩🇺🇸 ".repeat(600)),
        ("hangul_jamo", "\u{1112}\u{1161}\u{11ab} ".repeat(1024)),
    ];
    for (name, filler) in cases {
        let mut engine =
            ExpansionEngine::new(config.clone()).expect("benchmark engine should initialize");
        engine.process(InputEvent::Text(filler));
        group.bench_function(BenchmarkId::new("type_and_backspace", name), |bench| {
            bench.iter(|| {
                black_box(engine.process(InputEvent::Text("\u{301}".into())));
                black_box(engine.process(InputEvent::Backspace));
            });
        });
        group.bench_function(BenchmarkId::new("near_miss_trigger", name), |bench| {
            bench.iter(|| {
                black_box(engine.process(InputEvent::Text(":cafe\u{301}x".into())));
            });
        });
    }
    group.finish();
}

/// What a reload costs at the documented maximum: parse and validate the
/// TOML, then build the matcher, for a library at the snippet cap with the
/// trigger amplification (aliases, case, NFC/NFD variants) near its limit.
fn bench_reload_at_scale(c: &mut Criterion) {
    let mut group = c.benchmark_group("reload_parse_validate_build");
    group.sample_size(10);
    for count in [1_000, 10_000] {
        let text = adversarial_config_text(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &text, |bench, text| {
            bench.iter(|| {
                let config = Config::parse(text).expect("benchmark configuration should validate");
                black_box(
                    ExpansionEngine::new(config).expect("benchmark engine should initialize"),
                );
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_matcher,
    bench_long_buffer_keystroke,
    bench_matcher_construction,
    bench_unicode_worst_case,
    bench_reload_at_scale
);
criterion_main!(benches);
