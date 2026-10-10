//! `{{clipboard}}` support for the daemon.
//!
//! Reads the clipboard with `wl-paste` (wl-clipboard) through the same
//! bounded, shell-free command runner used for command-backed snippets: fixed
//! arguments, a minimal environment that passes only the Wayland session
//! variables, a short timeout, and the usual output cap. The clipboard is read
//! only for a snippet that uses the variable, and only when the user enabled
//! it (`settings.allow_clipboard`) and policy allows it. Contents are never
//! logged or cached.
//!
//! To keep `wl-paste` off the keystroke path, the engine may ask for a read a
//! little early, once the typed text can only become a `{{clipboard}}`
//! trigger. That value is held for at most [`PREFETCH_MAX_AGE`], handed to
//! the very next render, and then dropped; nothing monitors the clipboard.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use wayexpand_core::{
    run_command, ClipboardPrefetch, ClipboardReader, CommandConfig, CommandEnvironment,
};

/// How long a prefetched value may wait for its render before it is erased.
/// Typing the rest of a trigger takes well under this.
const PREFETCH_MAX_AGE: Duration = Duration::from_secs(2);

/// Upper bound on one `{{clipboard}}` read. A healthy `wl-paste` returns in a
/// few milliseconds; this only limits how long a stuck clipboard owner can
/// delay an expansion. Reading on demand (never caching) is deliberate, see
/// the module docs.
const CLIPBOARD_TIMEOUT_MS: u64 = 150;

fn wl_paste_command() -> CommandConfig {
    CommandConfig {
        action: None,
        program: "wl-paste".into(),
        args: vec![
            "--no-newline".into(),
            "--type".into(),
            "text/plain;charset=utf-8".into(),
        ],
        timeout_ms: CLIPBOARD_TIMEOUT_MS,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec!["WAYLAND_DISPLAY".into(), "XDG_RUNTIME_DIR".into()],
    }
}

type ReadClipboard = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The single in-flight or completed early read.
#[derive(Default)]
enum Prefetched {
    #[default]
    Empty,
    Pending,
    Ready(Instant, Option<String>),
}

#[derive(Default)]
struct Slot {
    /// Distinguishes reads, so a finished thread never overwrites or erases
    /// a newer read's value.
    generation: u64,
    value: Prefetched,
}

struct Shared {
    slot: Mutex<Slot>,
    ready: Condvar,
}

/// The `{{clipboard}}` reader and its prefetch hook, sharing one slot.
pub fn wl_paste_clipboard() -> (ClipboardReader, ClipboardPrefetch) {
    let command = wl_paste_command();
    prefetching_clipboard(
        Arc::new(move || run_command(&command).ok()),
        PREFETCH_MAX_AGE,
    )
}

fn prefetching_clipboard(
    read: ReadClipboard,
    max_age: Duration,
) -> (ClipboardReader, ClipboardPrefetch) {
    let shared = Arc::new(Shared {
        slot: Mutex::new(Slot::default()),
        ready: Condvar::new(),
    });
    let reader = {
        let shared = Arc::clone(&shared);
        let read = Arc::clone(&read);
        ClipboardReader(Arc::new(move || take_or_read(&shared, &read, max_age)))
    };
    let prefetch = ClipboardPrefetch(Arc::new(move || start_prefetch(&shared, &read, max_age)));
    (reader, prefetch)
}

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Slot> {
    shared
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn start_prefetch(shared: &Arc<Shared>, read: &ReadClipboard, max_age: Duration) {
    let generation = {
        let mut slot = lock(shared);
        match slot.value {
            Prefetched::Pending => return,
            Prefetched::Ready(at, _) if at.elapsed() < max_age => return,
            _ => {}
        }
        slot.generation = slot.generation.wrapping_add(1);
        slot.value = Prefetched::Pending;
        slot.generation
    };
    let worker_shared = Arc::clone(shared);
    let read = Arc::clone(read);
    let spawned = std::thread::Builder::new()
        .name("wayexpand-clipboard".into())
        .spawn(move || {
            let value = read();
            {
                let mut slot = lock(&worker_shared);
                if slot.generation != generation {
                    return;
                }
                slot.value = Prefetched::Ready(Instant::now(), value);
            }
            worker_shared.ready.notify_all();
            // Expiration is checked lazily by start_prefetch/take_or_read.
            // Do not keep an otherwise idle OS thread alive just to clear a
            // timestamped value.
        });
    if spawned.is_err() {
        let mut slot = lock(shared);
        if slot.generation == generation {
            slot.value = Prefetched::Empty;
        }
    }
}

/// Use the prefetched value if it is fresh (waiting for an in-flight read),
/// otherwise read synchronously. A prefetched value is used at most once.
fn take_or_read(shared: &Shared, read: &ReadClipboard, max_age: Duration) -> Option<String> {
    let mut slot = lock(shared);
    // The in-flight read is itself bounded by the wl-paste timeout; this is
    // a backstop if its thread is somehow delayed.
    let deadline = Instant::now() + Duration::from_millis(CLIPBOARD_TIMEOUT_MS * 2);
    while matches!(slot.value, Prefetched::Pending) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        slot = shared
            .ready
            .wait_timeout(slot, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
    match std::mem::take(&mut slot.value) {
        Prefetched::Ready(at, value) if at.elapsed() < max_age => return value,
        // Stale, never prefetched, or still pending past the backstop: the
        // pending read's result will be discarded by its generation check.
        _ => slot.generation = slot.generation.wrapping_add(1),
    }
    drop(slot);
    read()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_command_is_bounded_and_passes_only_wayland_variables() {
        let command = wl_paste_command();
        wayexpand_core::validate_command_config(&command).unwrap();
        assert_eq!(command.environment, CommandEnvironment::Minimal);
        assert_eq!(command.pass_env, ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]);
        assert!(command.timeout_ms <= 150);
        assert_eq!(command.cache_ms, 0);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting_read(delay: Duration) -> (ReadClipboard, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let read: ReadClipboard = Arc::new(move || {
            let call = counter.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::sleep(delay);
            Some(format!("value-{call}"))
        });
        (read, calls)
    }

    #[test]
    fn render_uses_a_prefetched_value_once() {
        let (read, calls) = counting_read(Duration::from_millis(30));
        let (reader, prefetch) = prefetching_clipboard(read, Duration::from_secs(5));
        (prefetch.0)();
        (prefetch.0)(); // already in flight: no second read
        assert_eq!((reader.0)().as_deref(), Some("value-1"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Consumed: the next render reads again.
        assert_eq!((reader.0)().as_deref(), Some("value-2"));
    }

    #[test]
    fn unused_prefetched_value_is_erased_after_max_age() {
        let (read, calls) = counting_read(Duration::ZERO);
        let (reader, prefetch) = prefetching_clipboard(read, Duration::from_millis(50));
        (prefetch.0)();
        std::thread::sleep(Duration::from_millis(200));
        // The stale value is gone; the render reads synchronously.
        assert_eq!((reader.0)().as_deref(), Some("value-2"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn render_without_prefetch_reads_synchronously() {
        let (read, calls) = counting_read(Duration::ZERO);
        let (reader, _prefetch) = prefetching_clipboard(read, Duration::from_secs(5));
        assert_eq!((reader.0)().as_deref(), Some("value-1"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
