//! End-to-end cost of receiving a large IBus surrounding-text update and
//! processing the next key. Run with `cargo bench -p wayexpand-backend-ibus
//! --bench surrounding-text`; output contains p50/p95 latency and allocated
//! bytes per update/key pair.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wayexpand_backend_ibus::{IbusEngineAdapter, IBUS_CAP_SURROUNDING_TEXT};
use wayexpand_core::{Config, ExpansionEngine};

struct MeasuringAllocator;

static MEASURE_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: Every operation is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for MeasuringAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURE_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: The caller provides a valid layout, which is forwarded as-is.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The pointer and layout are forwarded to the allocator that created them.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if MEASURE_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        // SAFETY: The pointer, original layout, and requested size are forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: MeasuringAllocator = MeasuringAllocator;

fn adapter() -> IbusEngineAdapter {
    let config = Config::parse("[[expansion]]\ntrigger = ';bench'\nreplacement = 'result'\n")
        .expect("static benchmark config is valid");
    let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
    adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
    adapter.set_content_type(0, 0);
    adapter.focus_in();
    adapter.enable();
    adapter
}

fn document(target_bytes: usize, mixed_unicode: bool) -> (String, u32) {
    let mut text = String::with_capacity(target_bytes);
    let mut characters = if mixed_unicode {
        "systemctl status café🙂; ".chars().cycle()
    } else {
        "systemctl status service-123; ".chars().cycle()
    };
    while text.len() < target_bytes {
        let character = characters.next().expect("the sample text is non-empty");
        if text.len() + character.len_utf8() > target_bytes {
            text.push('a');
        } else {
            text.push(character);
        }
    }
    let scalar_count = text.chars().count() as u32;
    (text, scalar_count)
}

fn percentile(samples: &mut [Duration], percentile: usize) -> Duration {
    samples.sort_unstable();
    samples[(samples.len() - 1) * percentile / 100]
}

fn run_case(label: &str, target_bytes: usize, mixed_unicode: bool) {
    const WARMUP: usize = 100;
    const SAMPLES: usize = 2_000;

    let (text, cursor) = document(target_bytes, mixed_unicode);
    let mut adapter = adapter();
    for _ in 0..WARMUP {
        adapter.set_surrounding_text(black_box(&text), cursor, cursor);
        black_box(adapter.process_key_event('x' as u32, 0, 0));
        adapter.reset();
    }

    let before = ALLOCATED_BYTES.load(Ordering::Relaxed);
    MEASURE_ALLOCATIONS.store(true, Ordering::Relaxed);
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        adapter.set_surrounding_text(black_box(&text), cursor, cursor);
        black_box(adapter.process_key_event('x' as u32, 0, 0));
        adapter.reset();
        samples.push(start.elapsed());
    }
    MEASURE_ALLOCATIONS.store(false, Ordering::Relaxed);
    let allocated = ALLOCATED_BYTES.load(Ordering::Relaxed) - before;
    let p50 = percentile(&mut samples, 50);
    let p95 = percentile(&mut samples, 95);
    let p99 = percentile(&mut samples, 99);
    println!(
        "{label:>18}: input={:>8} bytes, p50={:>8.2?}, p95={:>8.2?}, p99={:>8.2?}, allocated/update+key={:>6} bytes",
        text.len(),
        p50,
        p95,
        p99,
        allocated / SAMPLES as u64,
    );
}

fn main() {
    println!("IBus surrounding-text update + next-key processing");
    println!("2,000 samples after 100 warm-up iterations; allocations are requested bytes, not peak RSS.");
    for (kind, mixed_unicode) in [("ASCII", false), ("mixed UTF-8", true)] {
        run_case(&format!("{kind} / 128 B"), 128, mixed_unicode);
        run_case(&format!("{kind} / 100 KiB"), 100 * 1024, mixed_unicode);
        run_case(&format!("{kind} / 2 MiB"), 2 * 1024 * 1024, mixed_unicode);
    }
}
