//! Arbitrary input streams through a matcher with immediate, word-boundary,
//! case-propagating, and non-ASCII triggers. Every reported match must erase
//! only text that was actually typed since the last reset.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wayexpand_core::{Config, ExpansionEngine, InputEvent};

const CONFIG: &str = r#"
[[expansion]]
trigger = ":sig"
replacement = "signature"

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

[[expansion]]
trigger = ":東京"
replacement = "Tokyo"
"#;

fuzz_target!(|data: &[u8]| {
    let mut engine = ExpansionEngine::new(Config::parse(CONFIG).unwrap()).unwrap();
    // Text typed since the last event that clears the matcher.
    let mut typed = String::new();
    let mut bytes = data.iter().copied();
    while let Some(op) = bytes.next() {
        let event = match op % 8 {
            0 => InputEvent::Backspace,
            1 => InputEvent::Reset,
            2 => InputEvent::EndOfInput,
            3 => InputEvent::FocusChanged {
                sensitive: bytes.next().unwrap_or(0) % 2 == 0,
            },
            4 => InputEvent::Delimiter(
                [' ', '\n', '.', ',', '\t'][usize::from(bytes.next().unwrap_or(0)) % 5],
            ),
            _ => {
                let pick = bytes.next().unwrap_or(0);
                let alphabet = [
                    ':', 's', 'i', 'g', 'b', 't', 'w', ';', 'a', 'd', 'r', 'A', 'D', 'R', 'c', 'f',
                    'e', '\u{e9}', '\u{301}', '東', '京',
                ];
                InputEvent::Text(alphabet[usize::from(pick) % alphabet.len()].to_string())
            }
        };
        let clears_after_processing = matches!(
            &event,
            InputEvent::EndOfInput
                | InputEvent::Reset
                | InputEvent::FocusChanged { .. }
                | InputEvent::CompositionChanged { .. }
                | InputEvent::PauseChanged(_)
        );
        match &event {
            InputEvent::Text(text) => typed.push_str(text),
            InputEvent::Delimiter(character) => typed.push(*character),
            InputEvent::Backspace => {
                typed.pop();
            }
            _ => {}
        }
        for result in engine.process(event) {
            assert!(!result.matched_text.is_empty());
            assert!(
                typed.ends_with(&result.matched_text)
                    || result.reinsert_after.is_some_and(|delimiter| typed
                        .strip_suffix(delimiter)
                        .is_some_and(|before| before.ends_with(&result.matched_text))),
                "match {:?} erases text that was not typed ({typed:?})",
                result.matched_text
            );
            typed.clear();
        }
        if clears_after_processing {
            typed.clear();
        }
    }
});
