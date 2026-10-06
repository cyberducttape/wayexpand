//! Isolated matcher heap measurement (see docs/BENCHMARKS.md).
//!
//! Process RSS includes the harness and the parsed configuration, so this
//! counts only bytes allocated while building the `Matcher` from a
//! configuration's effective triggers (aliases, case propagation, NFC/NFD).
//!
//! ```sh
//! cargo run --locked --release -p wayexpand-core --example matcher_memory
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;
use wayexpand_core::{Config, Matcher};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let current = CURRENT.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(current, Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        CURRENT.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn config(name: &str, count: usize) -> Config {
    let mut text = String::new();
    for index in 0..count {
        let (trigger, aliases) = match name {
            "plain" => (format!(":snippet{index:05}"), String::new()),
            "amplified" => (
                format!(":{index:05}é"),
                format!(
                    "aliases = [\":a{index:05}_0\", \":a{index:05}_1\"]\npropagate_case = true\n"
                ),
            ),
            _ => (
                format!(":日本{index:05}"),
                format!(
                    "aliases = [\":ñ{index:05}ü\", \":Пр{index:05}\"]\npropagate_case = true\n"
                ),
            ),
        };
        text.push_str(&format!(
            "[[expansion]]\ntrigger = \"{trigger}\"\n{aliases}replacement = \"value\"\n\n"
        ));
    }
    Config::parse(&text).expect("measurement configuration should validate")
}

fn main() {
    let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    for name in ["plain", "amplified", "unicode"] {
        let config = config(name, 10_000);
        let triggers: Vec<String> = config
            .expansion
            .iter()
            .flat_map(|expansion| expansion.effective_triggers())
            .collect();
        let trigger_count = triggers.len();
        let base = CURRENT.load(Relaxed);
        PEAK.store(base, Relaxed);
        let start = Instant::now();
        let matcher = Matcher::new(triggers);
        let elapsed = start.elapsed();
        let steady = CURRENT.load(Relaxed) - base;
        let peak = PEAK.load(Relaxed) - base;
        std::hint::black_box(&matcher);
        println!(
            "{name:9} snippets=10000 triggers={trigger_count:6} build={elapsed:8.2?} \
             heap={:6.1} MiB peak={:6.1} MiB",
            mib(steady),
            mib(peak)
        );
    }
}
