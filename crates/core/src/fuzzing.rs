//! State-machine checks over arbitrary engine event sequences.
//!
//! Shared by the `engine_sequence` fuzz target (nightly, libFuzzer) and a
//! seeded stable test, so CI exercises the same invariants without a fuzzer.
//! Not part of the public API.

use crate::{Config, ExpansionEngine, ExpansionResult, InputEvent, KeyChord, WindowContext};
use unicode_segmentation::UnicodeSegmentation;

const CONFIG: &str = r#"
[settings]
undo_chord = "ctrl+z"

[[expansion]]
trigger = ":sig"
replacement = "signature"

[[expansion]]
trigger = ":s"
replacement = "short"

[[expansion]]
trigger = ":off"
replacement = "DISABLED"
enabled = false

[[expansion]]
trigger = ":app"
replacement = "APPONLY"
app_filter = ["app_id_exact:org.allowed"]

[[expansion]]
trigger = "btw"
replacement = "by the way"
match_mode = "word-boundary"

[[expansion]]
trigger = ";addr"
replacement = "{{cursor}}street"
propagate_case = true

[[expansion]]
trigger = ":café"
replacement = "coffee"
"#;

const ALPHABET: &[char] = &[
    ':', 's', 'i', 'g', 'o', 'f', 'a', 'p', 'b', 't', 'w', ';', 'd', 'r', 'A', 'D', 'R', 'c', 'é',
    'e', '\u{301}', ' ',
];

/// Whole triggers (and near misses), so state changes are frequently
/// followed by a complete match rather than only by random characters.
const WORDS: &[&str] = &[
    ":sig",
    ":s",
    ":off",
    ":app",
    "btw",
    ";addr",
    ";ADDR",
    ":café",
    ":cafe\u{301}",
    ":si",
];

fn window(app_id: &str) -> WindowContext {
    WindowContext {
        app_id: Some(app_id.into()),
        title: None,
        instance_id: None,
    }
}

/// Model of what the engine is allowed to do.
#[derive(Default)]
struct Model {
    /// Text typed since the matcher was last cleared.
    typed: String,
    sensitive: bool,
    composing: bool,
    paused: bool,
    allowed_window: bool,
    /// An expansion was applied and nothing has happened since.
    undo_available: bool,
}

impl Model {
    fn capture_enabled(&self) -> bool {
        !self.sensitive && !self.composing && !self.paused
    }
}

fn new_engine(model: &Model) -> ExpansionEngine {
    let mut engine = ExpansionEngine::new(Config::parse(CONFIG).expect("fixture config is valid"))
        .expect("fixture engine builds");
    if model.allowed_window {
        engine.set_current_window(Some(window("org.allowed")));
    }
    engine
}

fn check_result(model: &Model, result: &ExpansionResult) {
    assert!(
        model.capture_enabled(),
        "expansion while capture is off: {result:?}"
    );
    assert!(
        !result.insert.contains("DISABLED"),
        "disabled snippet fired"
    );
    assert!(
        !result.insert.contains("APPONLY") || model.allowed_window,
        "app-filtered snippet fired without its window identity"
    );
    assert!(!result.matched_text.is_empty());
    let typed = &model.typed;
    assert!(
        typed.ends_with(&result.matched_text)
            || result.reinsert_after.is_some_and(|delimiter| typed
                .strip_suffix(delimiter)
                .is_some_and(|before| before.ends_with(&result.matched_text))),
        "match {:?} erases text that was not typed ({typed:?})",
        result.matched_text
    );
}

/// Apply one event to the engine and the model, checking every result.
fn apply(engine: &mut ExpansionEngine, model: &mut Model, event: InputEvent) {
    match &event {
        InputEvent::FocusChanged { sensitive } => model.sensitive = *sensitive,
        InputEvent::CompositionChanged { active } => model.composing = *active,
        InputEvent::PauseChanged(paused) => model.paused = *paused,
        InputEvent::WindowChanged(window) => {
            model.allowed_window =
                window.as_ref().and_then(|window| window.app_id.as_deref()) == Some("org.allowed");
        }
        _ => {}
    }
    let clears = matches!(
        &event,
        InputEvent::EndOfInput
            | InputEvent::Reset
            | InputEvent::FocusChanged { .. }
            | InputEvent::CompositionChanged { .. }
            | InputEvent::PauseChanged(_)
    );
    if model.capture_enabled() {
        match &event {
            InputEvent::Text(text) => model.typed.push_str(text),
            InputEvent::Delimiter(character) => model.typed.push(*character),
            InputEvent::Backspace => {
                if let Some((start, _)) = model.typed.grapheme_indices(true).next_back() {
                    model.typed.truncate(start);
                }
            }
            _ => {}
        }
    }
    let results = engine.process(event);
    for result in &results {
        check_result(model, result);
        engine.commit_applied_expansion(result);
        // Mirror the transaction against the model's document: remove only
        // the matched suffix, preserve any earlier text, and keep a
        // terminating character that was folded into the replacement. The
        // previous model replaced the entire document with just the
        // terminator, producing false fuzz failures when a match followed
        // earlier buffered text.
        let prefix = result
            .reinsert_after
            .and_then(|delimiter| model.typed.strip_suffix(delimiter))
            .and_then(|before| before.strip_suffix(&result.matched_text))
            .or_else(|| model.typed.strip_suffix(&result.matched_text));
        let Some(prefix) = prefix else {
            unreachable!("check_result accepted a result that is not a typed suffix");
        };
        model.typed = prefix.to_owned();
        if let Some(delimiter) = result.reinsert_after {
            model.typed.push(delimiter);
        }
    }
    model.undo_available = results.last().is_some_and(|result| result.undoable);
    if clears {
        model.typed.clear();
    }
    assert!(
        engine.buffer_len_for_checks() <= engine.max_buffer_chars_for_checks(),
        "match buffer exceeded its bound"
    );
}

/// Feed `data` to an engine as an event sequence and assert the invariants.
/// Panics on a violation.
pub fn check_engine_sequence(data: &[u8]) {
    let undo = KeyChord::parse("ctrl+z").expect("fixture chord parses");
    let mut model = Model::default();
    let mut engine = new_engine(&model);
    let mut bytes = data.iter().copied();
    while let Some(op) = bytes.next() {
        let arg = bytes.next().unwrap_or(0);
        let undo_was_available = model.undo_available;
        model.undo_available = false;
        let event = match op % 14 {
            0 => InputEvent::Backspace,
            1 => InputEvent::Reset,
            2 => InputEvent::EndOfInput,
            3 => InputEvent::FocusChanged {
                sensitive: arg % 2 == 0,
            },
            4 => InputEvent::Delimiter([' ', '\n', '.', '\t'][usize::from(arg) % 4]),
            5 => InputEvent::CompositionChanged {
                active: arg % 2 == 0,
            },
            6 => InputEvent::PauseChanged(arg % 2 == 0),
            7 => InputEvent::WindowChanged(match arg % 3 {
                0 => None,
                1 => Some(window("org.allowed")),
                _ => Some(window("org.other")),
            }),
            8 => {
                // The undo chord, delivered like the daemon does: the key
                // event, then the undo attempt. Undo is offered only for the
                // expansion applied immediately before, never for an older
                // or invalidated one.
                let _ = engine.process(InputEvent::Key(undo.clone()));
                let prepared = engine.prepare_undo(&undo);
                assert!(
                    prepared.is_none() || (undo_was_available && model.capture_enabled()),
                    "undo offered for a stale expansion"
                );
                if let Some(result) = prepared {
                    engine.commit_undo(&result);
                }
                // Input sources follow every Ctrl/Alt/Super chord with a
                // Reset, because the application may have changed its text.
                apply(&mut engine, &mut model, InputEvent::Reset);
                continue;
            }
            9 => {
                // Configuration reload replaces the engine through the same
                // runtime-state hand-over the daemon and IBus use.
                let mut reloaded = new_engine(&Model::default());
                reloaded.inherit_runtime_state(&engine);
                engine = reloaded;
                model.typed.clear();
                continue;
            }
            10 => {
                // A whole trigger, typed one keystroke per event.
                for character in WORDS[usize::from(arg) % WORDS.len()].chars() {
                    model.undo_available = false;
                    apply(
                        &mut engine,
                        &mut model,
                        InputEvent::Text(character.to_string()),
                    );
                }
                continue;
            }
            _ => InputEvent::Text(ALPHABET[usize::from(arg) % ALPHABET.len()].to_string()),
        };
        apply(&mut engine, &mut model, event);
    }
}

#[cfg(test)]
mod tests {
    /// The same invariants as the fuzz target, over seeded pseudo-random
    /// sequences so stable CI exercises them too.
    #[test]
    fn engine_event_sequences_hold_their_invariants() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..1_000 {
            let length = 16 + (state % 400) as usize;
            let mut data = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                data.push((state >> 24) as u8);
            }
            super::check_engine_sequence(&data);
        }
    }

    #[test]
    fn engine_sequence_regression_from_fuzz_artifact() {
        // Replayed from the engine_sequence artifact produced by the hosted
        // fuzz job for commit 8644844.
        super::check_engine_sequence(&[0x7a, 0x81, 0x7a, 0x81, 0x7a, 0x81, 0x00, 0x00, 0x37, 0x5b]);
    }
}
