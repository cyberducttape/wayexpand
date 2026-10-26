//! Cost of one GUI editing transaction on a large library: the editor clones
//! the whole configuration, edits the copy, and the save path validates it
//! before anything is written. This keeps the copy-then-validate model
//! honest at the documented 10,000-snippet maximum.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::{
    fs,
    hint::black_box,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use wayexpand_core::{Config, ExpansionEngine};

fn library_source(count: usize) -> String {
    let mut text = String::with_capacity(count * 160);
    for index in 0..count {
        text.push_str(&format!(
            "[[expansion]]\nid = \"00000000-0000-4000-8000-{index:012x}\"\n\
             trigger = \":snippet{index:05}\"\n\
             replacement = \"Replacement text {index} with a sentence of ordinary length.\"\n\
             description = \"snippet {index}\"\ntags = [\"work\", \"email\"]\n\n"
        ));
    }
    text
}

fn library(count: usize) -> Config {
    Config::parse(&library_source(count)).expect("benchmark configuration should validate")
}

fn private_temp_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-reload-bench-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("create private benchmark directory");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("restrict benchmark directory permissions");
    path
}

fn write_private(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).expect("write benchmark configuration");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("restrict benchmark configuration permissions");
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

    let mut reload_group = c.benchmark_group("config_reload");
    for count in [100, 1_000, 5_000, 10_000] {
        let directory = private_temp_directory();
        let path = directory.join("expansions.toml");
        let source = library_source(count);
        write_private(&path, &source);
        reload_group.bench_function(BenchmarkId::new("single_secure_read", count), |bench| {
            bench.iter(|| {
                black_box(Config::load_versioned(&path).expect("configuration should validate"));
            });
        });
        reload_group.bench_function(
            BenchmarkId::new("revision_only_second_read", count),
            |bench| {
                bench.iter(|| {
                    let loaded = Config::load_versioned(&path).expect("load should validate");
                    assert!(Config::source_matches_revision(&path, &loaded.revision)
                        .expect("revision check should read securely"));
                    black_box(loaded);
                });
            },
        );
        reload_group.bench_function(
            BenchmarkId::new("file_load_validation_and_engine_build", count),
            |bench| {
                bench.iter(|| {
                    let config = Config::load(&path).expect("reload configuration should validate");
                    black_box(ExpansionEngine::new(config).expect("engine should build"));
                });
            },
        );
        fs::remove_dir_all(directory).expect("remove benchmark directory");
    }
    reload_group.finish();
}

criterion_group!(benches, bench_config_edit);
criterion_main!(benches);
