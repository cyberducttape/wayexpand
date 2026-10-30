//! Count allocations in the warmed deferred ASCII input path.
//!
//! ```sh
//! cargo run --locked --release -p wayexpand-core --example input_path_allocations
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use wayexpand_core::{Config, ExpansionEngine, InputEvent, PendingExpansionResult};

struct Counting;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && TRACK.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && TRACK.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, size) };
        if !new_pointer.is_null() && TRACK.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        new_pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() {
    const WARMUP_EVENTS: usize = 4_096;
    const MEASURED_EVENTS: usize = 100_000;

    let config = Config::parse(
        r#"[[expansion]]
trigger = ":never"
replacement = "unused""#,
    )
    .expect("measurement configuration should validate");
    let mut engine = ExpansionEngine::new(config).expect("measurement engine should build");
    let mut pending: Vec<PendingExpansionResult> = Vec::new();

    // Grow and settle the bounded input ring before measuring, then reset the
    // matcher while retaining its allocated storage.
    for _ in 0..WARMUP_EVENTS {
        engine.process_deferred_char_into('x', &mut pending);
    }
    engine.process_deferred_into(InputEvent::Reset, &mut pending);

    ALLOCATIONS.store(0, Relaxed);
    TRACK.store(true, Relaxed);
    for _ in 0..MEASURED_EVENTS {
        engine.process_deferred_char_into('x', &mut pending);
    }
    TRACK.store(false, Relaxed);

    let allocations = ALLOCATIONS.load(Relaxed);
    std::hint::black_box(&engine);
    println!(
        "warmed deferred ASCII input: {allocations} allocations across {MEASURED_EVENTS} events ({:.6} allocations/event)",
        allocations as f64 / MEASURED_EVENTS as f64
    );
    assert_eq!(
        allocations, 0,
        "the warmed nonmatching ASCII path allocated"
    );
}
