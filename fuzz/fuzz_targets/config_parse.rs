//! Untrusted configuration text: parsing and validation must reject bad
//! input with an error, never a panic, and anything accepted must build an
//! engine.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wayexpand_core::{Config, ExpansionEngine};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(config) = Config::parse(text) {
        // Error summaries are shown to users and must never panic either.
        if let Err(error) = config.validate() {
            let _ = error.safe_summary();
        } else {
            ExpansionEngine::new(config).expect("a validated config builds an engine");
        }
    } else if let Err(error) = Config::parse(text) {
        let _ = error.safe_summary();
    }
});
