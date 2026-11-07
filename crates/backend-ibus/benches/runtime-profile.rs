//! End-to-end IBus engine resource profile.
//!
//! Run with `cargo bench -p wayexpand-backend-ibus --bench runtime-profile`.
//! Linux runs also report process thread count and RSS/PSS from procfs. Other
//! platforms report those OS-level measurements as unavailable.

use std::{
    hint::black_box,
    thread,
    time::{Duration, Instant},
};
use wayexpand_backend_ibus::{IbusEngineAdapter, IBUS_CAP_SURROUNDING_TEXT};
use wayexpand_core::{Config, ExpansionEngine};

const CONTEXT_COUNTS: [usize; 3] = [1, 4, 16];
const RESOURCE_REPETITIONS: usize = 5;
const LATENCY_SAMPLES: usize = 2_000;

fn text_config() -> Config {
    Config::parse("[[expansion]]\ntrigger = ';profile'\nreplacement = 'WayExpand profile'\n")
        .unwrap()
}

fn async_config() -> Config {
    Config::parse(
        "[[expansion]]\ntrigger = ';profile'\nreplacement = ''\n\
         [expansion.command]\nprogram = '/usr/bin/true'\nargs = []\n",
    )
    .unwrap()
}

fn adapters(config: &Config, contexts: usize) -> Vec<IbusEngineAdapter> {
    (0..contexts)
        .map(|_| IbusEngineAdapter::new(ExpansionEngine::new(config.clone()).unwrap()))
        .collect()
}

#[derive(Default)]
struct ProcessMemory {
    rss_kib: Option<u64>,
    pss_kib: Option<u64>,
}

fn process_threads() -> Option<usize> {
    std::fs::read_dir("/proc/self/task")
        .ok()
        .map(|entries| entries.count())
}

fn process_memory() -> ProcessMemory {
    let Ok(contents) = std::fs::read_to_string("/proc/self/smaps_rollup") else {
        return ProcessMemory::default();
    };
    let mut memory = ProcessMemory::default();
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        let Some(value) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        match name {
            "Rss:" => memory.rss_kib = Some(value),
            "Pss:" => memory.pss_kib = Some(value),
            _ => {}
        }
    }
    memory
}

fn resources() -> String {
    let memory = process_memory();
    format!(
        "threads={} rss_kib={} pss_kib={}",
        process_threads().map_or_else(|| "n/a".into(), |value| value.to_string()),
        memory
            .rss_kib
            .map_or_else(|| "n/a".into(), |value| value.to_string()),
        memory
            .pss_kib
            .map_or_else(|| "n/a".into(), |value| value.to_string()),
    )
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    sorted[((sorted.len() - 1) * percentile).div_ceil(100)]
}

fn report_latency(samples: &mut [u128]) -> String {
    samples.sort_unstable();
    format!(
        "p50={}ns p95={}ns p99={}ns",
        percentile(samples, 50),
        percentile(samples, 95),
        percentile(samples, 99)
    )
}

fn profile_contexts(label: &str, config: &Config) {
    for contexts in CONTEXT_COUNTS {
        let mut elapsed = Vec::with_capacity(RESOURCE_REPETITIONS);
        let before = resources();
        for _ in 0..RESOURCE_REPETITIONS {
            let start = Instant::now();
            let created = adapters(config, contexts);
            black_box(&created);
            elapsed.push(start.elapsed().as_micros());
            drop(created);
            // Let closed worker queues finish waking and exit before the next
            // sample so measurements describe live contexts, not teardown.
            thread::sleep(Duration::from_millis(20));
        }
        let mut sorted = elapsed;
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        let live = adapters(config, contexts);
        let workers: usize = live
            .iter()
            .map(|adapter| {
                let metrics = adapter.engine().command_metrics();
                metrics.command_worker_threads + metrics.hotkey_worker_threads
            })
            .sum();
        println!(
            "context_profile kind={label} contexts={contexts} create_median={median}us reported_worker_threads={workers} before=[{before}] after=[{}]",
            resources()
        );
        drop(live);
        thread::sleep(Duration::from_millis(100));
    }
}

fn profile_reload(config: &Config) {
    for contexts in CONTEXT_COUNTS {
        let mut engines = adapters(config, contexts);
        let mut samples = Vec::with_capacity(25);
        for _ in 0..25 {
            let start = Instant::now();
            for adapter in &mut engines {
                adapter.replace_config(config.clone()).unwrap();
            }
            samples.push(start.elapsed().as_nanos());
        }
        println!(
            "config_reload kind=command contexts={contexts} samples={} {} resources=[{}]",
            samples.len(),
            report_latency(&mut samples),
            resources()
        );
    }
}

fn profile_typing_latency(config: &Config, label: &str) {
    let mut adapter = adapters(config, 1).pop().unwrap();
    adapter.focus_in();
    adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
    adapter.set_content_type(0, 0);
    let mut samples = Vec::with_capacity(LATENCY_SAMPLES);
    for index in 0..LATENCY_SAMPLES {
        adapter.focus_in();
        let start = Instant::now();
        let result = adapter.process_key_event('x' as u32, 0, 0);
        samples.push(start.elapsed().as_nanos());
        black_box(result);
        if index % 100 == 99 {
            adapter.set_content_type(0, 0);
        }
    }
    println!(
        "typing_latency kind={label} samples={} {} resources=[{}]",
        samples.len(),
        report_latency(&mut samples),
        resources()
    );
}

fn main() {
    let text = text_config();
    let command = async_config();
    println!("ibus_runtime_profile workers=4_command+1_hotkey_per_async_context");
    profile_contexts("text_only", &text);
    profile_contexts("async_required", &command);
    profile_reload(&command);
    profile_typing_latency(&text, "text_only");

    // Typing latency while the runtime is at its largest measured context
    // count helps expose scheduler contention from per-context workers.
    let loaded = adapters(&command, 16);
    profile_typing_latency(&text, "with_16_async_contexts");
    drop(loaded);
    thread::sleep(Duration::from_millis(100));
    println!("final_resources [{}]", resources());
}
