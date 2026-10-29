//! Template rendering costs for ordinary substitutions, maximum include
//! depth, and output near the configured byte limit.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::{collections::HashMap, hint::black_box, sync::Arc};
use wayexpand_core::{
    limits::MAX_RENDERED_BYTES, render_template, render_template_preview, CompiledTemplate,
    SnippetLibrary, TemplateContext,
};

fn bench_template_render(c: &mut Criterion) {
    let mut typical_group = c.benchmark_group("template_render_typical");
    let context = TemplateContext {
        username: "wayexpand-user".into(),
        hostname: "workstation".into(),
        unix_timestamp: 1_798_000_000,
        ..TemplateContext::default()
    };
    let template = "Hello {{username}} on {{hostname}}.\nStarted at {{local_datetime}}.";
    typical_group.throughput(Throughput::Bytes(template.len() as u64));
    typical_group.bench_function("builtins", |bench| {
        bench.iter(|| {
            black_box(render_template(black_box(template), black_box(&context)).unwrap());
        });
    });
    // What the engine does per expansion: the template was compiled once
    // during validation.
    let compiled = CompiledTemplate::compile(template).unwrap();
    typical_group.bench_function("builtins_precompiled", |bench| {
        bench.iter(|| {
            black_box(
                black_box(&compiled)
                    .render_with_cursor(black_box(&context))
                    .unwrap(),
            );
        });
    });
    typical_group.finish();

    let mut snippets = HashMap::new();
    for level in 1..8 {
        snippets.insert(
            format!(":level{level}"),
            Arc::<str>::from(format!("level{level} {{snippet::level{}}}", level + 1)),
        );
    }
    snippets.insert(":level8".into(), Arc::<str>::from("leaf {{username}}"));
    let nested_context = TemplateContext {
        snippets: Arc::new(SnippetLibrary::from_iter(snippets)),
        username: "wayexpand-user".into(),
        ..TemplateContext::default()
    };
    c.bench_function("template_render_include_depth_8", |bench| {
        bench.iter(|| {
            black_box(
                render_template("root {{snippet::level1}}", black_box(&nested_context)).unwrap(),
            );
        });
    });

    let near_limit_template = "x".repeat(MAX_RENDERED_BYTES);
    let mut limit_group = c.benchmark_group("template_render_output_limit");
    limit_group.sample_size(10);
    limit_group.throughput(Throughput::Bytes(MAX_RENDERED_BYTES as u64));
    limit_group.bench_with_input(
        BenchmarkId::new("full_render_bytes", MAX_RENDERED_BYTES),
        &near_limit_template,
        |bench, template| {
            bench.iter(|| {
                black_box(render_template(black_box(template), black_box(&context)).unwrap());
            });
        },
    );
    limit_group.finish();

    let mut preview_group = c.benchmark_group("template_render_bounded_preview");
    preview_group.sample_size(10);
    // A literal 512-byte preview examines at most one byte beyond the output
    // budget to determine that truncation is needed.
    preview_group.throughput(Throughput::Bytes(513));
    preview_group.bench_with_input(
        BenchmarkId::new("bounded_preview_bytes", MAX_RENDERED_BYTES),
        &near_limit_template,
        |bench, template| {
            bench.iter(|| {
                black_box(
                    render_template_preview(black_box(template), black_box(&context), 512).unwrap(),
                );
            });
        },
    );
    preview_group.finish();
}

criterion_group!(benches, bench_template_render);
criterion_main!(benches);
