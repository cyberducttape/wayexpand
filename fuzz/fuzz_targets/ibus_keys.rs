//! Arbitrary IBus key events, content types, and surrounding-text reports.
//! A simulated client applies the engine's actions; the engine must never ask
//! it to delete more text than exists before the cursor.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wayexpand_backend_ibus::{IbusAction, IbusEngineAdapter, IBUS_CAP_SURROUNDING_TEXT};
use wayexpand_core::{Config, ExpansionEngine};

const CONFIG: &str = r#"
[[expansion]]
trigger = ":sig"
replacement = "signature"

[[expansion]]
trigger = "btw"
replacement = "by the way"
match_mode = "word-boundary"

[[expansion]]
trigger = ";αβ"
replacement = "alpha beta"
"#;

// One adapter for all runs: building one starts command worker threads, and
// tearing them down per input made this target roughly 500 times slower.
// Each run starts from a reset, refocused field.
static ADAPTER: std::sync::OnceLock<std::sync::Mutex<IbusEngineAdapter>> =
    std::sync::OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let mut adapter = ADAPTER
        .get_or_init(|| {
            std::sync::Mutex::new(IbusEngineAdapter::new(
                ExpansionEngine::new(Config::parse(CONFIG).unwrap()).unwrap(),
            ))
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    adapter.focus_out();
    adapter.reset();
    adapter.focus_in();
    adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
    adapter.set_content_type(0, 0);
    let mut document: Vec<char> = Vec::new();
    for chunk in data.chunks(9) {
        if chunk.len() < 9 {
            break;
        }
        let keyval = u32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]);
        let state = u32::from_le_bytes([chunk[5], chunk[6], chunk[7], chunk[8]]);
        match chunk[0] % 8 {
            0 => adapter.set_content_type(keyval % 16, state),
            1 => adapter.focus_in(),
            2 => adapter.reset(),
            3 => {
                // A stale or wrong report: the engine must not trust it.
                let text: String = document.iter().rev().take(usize::from(chunk[1] % 8)).collect();
                adapter.set_surrounding_text(&text, keyval % 16, state % 16);
            }
            _ => {
                let text: String = document.iter().collect();
                let cursor = document.len() as u32;
                adapter.set_surrounding_text(&text, cursor, cursor);
                // Bias towards printable keys so triggers actually occur.
                let keyval = if chunk[0] % 2 == 0 {
                    u32::from(b":sigbtw ;"[usize::from(chunk[1]) % 9])
                } else {
                    keyval
                };
                let state = if chunk[0] % 4 == 0 { 0 } else { state };
                let result = adapter.process_key_event(keyval, 0, state);
                for action in &result.actions {
                    match action {
                        IbusAction::DeleteSurroundingText { nchars } => {
                            let count = *nchars as usize;
                            assert!(count <= document.len(), "deleted {count} of {} chars", document.len());
                            document.truncate(document.len() - count);
                        }
                        IbusAction::CommitText(text) => document.extend(text.chars()),
                    }
                }
            }
        }
    }
});
