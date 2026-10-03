//! Untrusted app-filter expressions (including globs) against untrusted
//! window identities. Input is split at the first NUL into the filter and
//! the focused app ID; matching must never panic, and a filtered snippet
//! must never fire without a known window.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wayexpand_core::{AppFilter, Config, ExpansionEngine, InputEvent, WindowContext};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let (filter, app_id) = text.split_once('\0').unwrap_or((text, ""));
    let _ = AppFilter::parse(filter);
    let config = format!(
        "[organization]\nallow_weak_app_filters = true\n\
         [[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [{filter:?}]\n"
    );
    let Ok(config) = Config::parse(&config) else {
        return;
    };
    let Ok(mut engine) = ExpansionEngine::new(config) else {
        return;
    };
    // Fail closed: no window means no match.
    assert!(engine.process(InputEvent::Text(":x".into())).is_empty());
    engine.process(InputEvent::Reset);
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some(app_id.to_owned()),
        title: Some(app_id.to_owned()),
    })));
    let _ = engine.process(InputEvent::Text(":x".into()));
});
