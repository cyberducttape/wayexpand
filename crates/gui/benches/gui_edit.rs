//! Measure the actual hot editor feedback lookup and the GUI save pipeline
//! across the documented maximum library sizes.

#![allow(dead_code)]

use std::{fs, hint::black_box, sync::Arc};

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use wayexpand_core::Config;

#[path = "../src/editor.rs"]
#[allow(unused_imports)]
mod editor;
#[path = "../src/preview.rs"]
mod preview;

#[path = "../src/library.rs"]
mod library;
#[path = "../src/persistence.rs"]
mod persistence;

fn library_config(count: usize) -> Config {
    let mut text = String::with_capacity(count * 180);
    for index in 0..count {
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":snippet{index:05}\"\n\
             replacement = \"Replacement text {index} with ordinary content.\"\n\
             description = \"snippet {index}\"\ntags = [\"work\", \"email\"]\n\n"
        ));
    }
    Config::parse(&text).expect("benchmark configuration should validate")
}

fn bench_editor_feedback(c: &mut Criterion) {
    let mut group = c.benchmark_group("gui_draft_trigger_feedback");
    for count in [100, 1_000, 5_000, 10_000] {
        let config = library_config(count);
        let index = library::SearchIndex::new(&config);
        let query = format!(":snippet{:05}", count - 1);
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &query,
            |bench, query| {
                bench.iter(|| black_box(index.has_trigger_conflict(black_box(query), Some(0))));
            },
        );
    }
    group.finish();
}

fn bench_gui_typing_preview(c: &mut Criterion) {
    let mut group = c.benchmark_group("gui_typing_preview_edit");
    for count in [100, 1_000, 5_000, 10_000] {
        let config = library_config(count);
        let expansion = config.expansion[0].clone();
        let mut draft = editor::Draft::from_expansion(&expansion);
        let snippets = config.includable_snippets();
        let input = expansion.trigger.clone();
        group.bench_function(BenchmarkId::from_parameter(count), |bench| {
            bench.iter(|| {
                // Model one replacement-field keystroke followed by the
                // editor's actual plain-text preview path. The library and
                // trigger input are prepared as they are in GuiApp; cloning
                // the shared snippet index is an Arc increment, not a library
                // rebuild. Restore the draft without reallocating so each
                // sample measures the same single-character edit.
                draft.replacement.push('x');
                let result = preview::render_with_library_snippets(
                    Some(&expansion),
                    &config.settings,
                    &config.organization,
                    Some(&draft),
                    &input,
                    "",
                    Some(Arc::clone(&snippets)),
                );
                draft.replacement.pop();
                black_box(result.expect("benchmark preview should render"));
            });
        });
    }
    group.finish();
}

fn bench_gui_save(c: &mut Criterion) {
    let mut group = c.benchmark_group("gui_full_save");
    for count in [100, 1_000, 5_000, 10_000] {
        let config = library_config(count);
        let directory = tempfile::tempdir().expect("private benchmark directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                .expect("set private benchmark directory permissions");
        }
        let path = directory.path().join("expansions.toml");
        let initial = toml_edit::ser::to_document(&config)
            .expect("serialize benchmark config")
            .to_string();
        fs::write(&path, initial).expect("write initial benchmark config");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("set private benchmark config permissions");
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &config,
            |bench, source_config| {
                bench.iter_batched(
                    || {
                        let loaded =
                            Config::load_versioned(&path).expect("load benchmark revision");
                        let source = fs::read_to_string(&path).expect("read benchmark document");
                        let document = persistence::read_config_document(&source)
                            .expect("parse benchmark document");
                        (loaded.revision, document)
                    },
                    |(revision, base_document)| {
                        let mut candidate = source_config.clone();
                        candidate.expansion[0].replacement =
                            "Edited replacement in timed GUI save pipeline.".into();
                        candidate.validate().expect("validate edited config");
                        let replacement = toml_edit::ser::to_document(&candidate)
                            .expect("serialize edited config");
                        let document =
                            persistence::merge_config_document(base_document, replacement);
                        Config::save_atomic_text_if_revision_matches(
                            path.clone(),
                            &document.to_string(),
                            &revision,
                        )
                        .expect("atomically save benchmark config");
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

/// A library whose replacements are long: templates, signatures, canned
/// replies. Index construction lowercases every replacement.
fn long_replacement_config(count: usize, replacement_bytes: usize) -> Config {
    let body = "Ordinary reply text. ".repeat(replacement_bytes / 21 + 1);
    let mut text = String::with_capacity(count * (replacement_bytes + 120));
    for index in 0..count {
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \":snippet{index:05}\"\n\
             replacement = \"{index} {}\"\ndescription = \"snippet {index}\"\n\n",
            &body[..replacement_bytes]
        ));
    }
    Config::parse(&text).expect("benchmark configuration should validate")
}

/// Rebuilding the sidebar's search index: on load and after every save.
fn bench_search_index_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("gui_search_index_build");
    group.sample_size(20);
    for count in [100, 1_000, 10_000] {
        let config = library_config(count);
        group.bench_with_input(
            BenchmarkId::new("short", count),
            &config,
            |bench, config| {
                bench.iter(|| black_box(library::SearchIndex::new(black_box(config))));
            },
        );
    }
    // The largest library the 16 MiB configuration limit admits at this
    // replacement size.
    for count in [1_000, 10_000] {
        let config = long_replacement_config(count, 1_000);
        group.bench_with_input(BenchmarkId::new("1kB", count), &config, |bench, config| {
            bench.iter(|| black_box(library::SearchIndex::new(black_box(config))));
        });
    }
    group.finish();
}

/// One keystroke in the sidebar search box.
fn bench_search_keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("gui_search_keystroke");
    for count in [100, 1_000, 10_000] {
        let config = library_config(count);
        let index = library::SearchIndex::new(&config);
        let all_fields = library::SearchFields {
            triggers: true,
            descriptions: true,
            tags: true,
            replacements: true,
        };
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &config,
            |bench, config| {
                bench.iter(|| {
                    black_box(index.visible_indices(
                        config,
                        black_box("ordinary"),
                        None,
                        all_fields,
                    ))
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_editor_feedback,
    bench_gui_typing_preview,
    bench_gui_save,
    bench_search_index_build,
    bench_search_keystroke
);
criterion_main!(benches);
