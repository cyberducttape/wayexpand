//! A scripted IBus client for `scripts/test-ibus-e2e.sh`.
//!
//! It talks to a real, private ibus-daemon exactly the way a toolkit does:
//! it creates an input context, reports capabilities, content type and
//! surrounding text, forwards key events, and applies the `CommitText` and
//! `DeleteSurroundingText` signals it receives to its own text model. Each
//! scenario checks the resulting text, so protocol mismatches between the
//! WayExpand engine and the real daemon fail here instead of on a desktop.
//!
//! Usage: wx-e2e-ibus <status-file> [--expect-active] [--single-expansion]
//!        (IBUS_ADDRESS must be set)
//! Prints one "pass|fail <scenario> <detail>" line per scenario and exits
//! non-zero if any scenario failed.

use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use zbus::{
    blocking::{Connection, MessageIterator},
    zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value},
};

const IBUS: &str = "org.freedesktop.IBus";
const IBUS_PATH: &str = "/org/freedesktop/IBus";
const CONTEXT: &str = "org.freedesktop.IBus.InputContext";
const CAP_PREEDIT: u32 = 1 << 0;
const CAP_FOCUS: u32 = 1 << 3;
const CAP_SURROUNDING_TEXT: u32 = 1 << 5;
const PURPOSE_FREE_FORM: u32 = 0;
const PURPOSE_PASSWORD: u32 = 8;
const RELEASE_MASK: u32 = 1 << 30;
const TRIGGER: &str = ";e2e";
const REPLACEMENT: &str = "expanded";

enum Event {
    Commit(String),
    Delete(i32, u32),
}

fn ibus_text(text: &str) -> Value<'static> {
    let empty: HashMap<String, Value<'static>> = HashMap::new();
    let attrs = StructureBuilder::new()
        .add_field("IBusAttrList")
        .add_field(empty.clone())
        .add_field(Vec::<Value<'static>>::new())
        .build()
        .expect("valid attribute list");
    Value::from(
        StructureBuilder::new()
            .add_field("IBusText")
            .add_field(empty)
            .add_field(text.to_owned())
            .add_field(Value::from(attrs))
            .build()
            .expect("valid IBusText"),
    )
}

fn text_of(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Value(inner) => text_of(inner),
        Value::Structure(structure) => match structure.fields() {
            [Value::Str(name), _, Value::Str(text), ..] if name.as_str() == "IBusText" => {
                Some(text.to_string())
            }
            _ => None,
        },
        _ => None,
    }
}

/// One input context with its own document, like a text field.
struct Field<'a> {
    connection: &'a Connection,
    path: OwnedObjectPath,
    events: &'a mpsc::Receiver<(String, Event)>,
    text: Vec<char>,
    cursor: usize,
    reports_text: bool,
}

impl<'a> Field<'a> {
    fn call<B>(&self, method: &str, body: &B) -> Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.connection
            .call_method(Some(IBUS), self.path.as_str(), Some(CONTEXT), method, body)
            .with_context(|| format!("InputContext.{method}"))
    }

    fn set_content_type(&self, purpose: u32) -> Result<()> {
        // Toolkits set the input context's ContentType property; ibus-daemon
        // forwards it to the engine only when it changes.
        self.connection
            .call_method(
                Some(IBUS),
                self.path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &(CONTEXT, "ContentType", Value::from((purpose, 0_u32))),
            )
            .context("InputContext ContentType")?;
        Ok(())
    }

    fn report_text(&self) -> Result<()> {
        if self.reports_text {
            let text: String = self.text.iter().collect();
            let cursor = self.cursor as u32;
            self.call("SetSurroundingText", &(ibus_text(&text), cursor, cursor))?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.text.clear();
        self.cursor = 0;
        self.report_text()
    }

    /// Apply signals for this context until none arrive for `quiet`.
    fn drain(&mut self, quiet: Duration) {
        while let Ok((path, event)) = self.events.recv_timeout(quiet) {
            if path != self.path.as_str() {
                continue;
            }
            match event {
                Event::Commit(text) => {
                    for character in text.chars() {
                        self.text.insert(self.cursor, character);
                        self.cursor += 1;
                    }
                }
                Event::Delete(offset, count) => {
                    let start = (self.cursor as i64 + offset as i64).max(0) as usize;
                    let end = (start + count as usize).min(self.text.len());
                    self.text.drain(start..end);
                    self.cursor = start;
                }
            }
            let _ = self.report_text();
        }
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        for character in text.chars() {
            let keyval = character as u32;
            let handled: bool = self
                .call("ProcessKeyEvent", &(keyval, 0_u32, 0_u32))?
                .body()
                .deserialize()?;
            self.call("ProcessKeyEvent", &(keyval, 0_u32, RELEASE_MASK))?;
            self.drain(Duration::from_millis(80));
            if !handled {
                // The toolkit applies keys the input method did not take.
                self.text.insert(self.cursor, character);
                self.cursor += 1;
                self.report_text()?;
            }
        }
        self.drain(Duration::from_millis(150));
        Ok(())
    }

    fn contents(&self) -> String {
        self.text.iter().collect()
    }
}

fn create_field<'a>(
    connection: &'a Connection,
    events: &'a mpsc::Receiver<(String, Event)>,
    name: &str,
    capabilities: u32,
    reports_text: bool,
) -> Result<Field<'a>> {
    let path: OwnedObjectPath = connection
        .call_method(
            Some(IBUS),
            IBUS_PATH,
            Some(IBUS),
            "CreateInputContext",
            &(name,),
        )
        .context("CreateInputContext")?
        .body()
        .deserialize()?;
    let field = Field {
        connection,
        path,
        events,
        text: Vec::new(),
        cursor: 0,
        reports_text,
    };
    field.call("SetCapabilities", &(capabilities,))?;
    Ok(field)
}

struct Results {
    failed: bool,
}

impl Results {
    fn check(&mut self, name: &str, passed: bool, detail: impl AsRef<str>) {
        println!(
            "{} {name} {}",
            if passed { "pass" } else { "fail" },
            detail.as_ref()
        );
        self.failed |= !passed;
    }
}

fn select_engine(connection: &Connection, field: &Field<'_>, expect_active: bool) -> Result<()> {
    field.call("FocusIn", &())?;
    if !expect_active {
        // A mismatched component name makes this call time out, because
        // ibus-daemon waits for the component's bus name before CreateEngine.
        connection
            .call_method(
                Some(IBUS),
                IBUS_PATH,
                Some(IBUS),
                "SetGlobalEngine",
                &("wayexpand",),
            )
            .context("SetGlobalEngine(wayexpand)")?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let engine: OwnedValue = connection
            .call_method(Some(IBUS), IBUS_PATH, Some(IBUS), "GetGlobalEngine", &())?
            .body()
            .deserialize()?;
        // IBusEngineDesc serializes its name as the third field.
        let name = match &*engine {
            Value::Structure(structure) => match structure.fields().get(2) {
                Some(Value::Str(name)) => name.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        if name == "wayexpand" {
            if expect_active {
                // GNOME's Shell-selected global descriptor can precede the
                // per-context engine binding. Reapply that already-verified
                // selection after FocusIn so IBus attaches the selected
                // engine to this test context, as it does when a toolkit
                // context becomes active.
                connection
                    .call_method(
                        Some(IBUS),
                        IBUS_PATH,
                        Some(IBUS),
                        "SetGlobalEngine",
                        &(name.as_str(),),
                    )
                    .context("SetGlobalEngine(wayexpand) for active context")?;
            }
            return Ok(());
        }
        if Instant::now() > deadline {
            bail!("global engine is {name:?}, not wayexpand");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Wait for content-free engine telemetry to confirm that IBus has created
/// and focused the actual engine context. `GetGlobalEngine` can return the
/// selected descriptor before the first input context is fully attached.
fn wait_for_focused_engine(status_file: &Path, after_focus_sequence: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reading) = wayexpand_backend_ibus::telemetry::read_status_at(status_file) {
            let status = reading.status;
            if status.engines > 0
                && status.recent_fields.iter().any(|field| {
                    field.sequence > after_focus_sequence
                        && field.focused
                        && field.content_type
                            == wayexpand_backend_ibus::telemetry::IbusContentTypeState::Normal
                        && field.surrounding_capability
                })
            {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            bail!("IBus selected WayExpand but no focused, safe surrounding-text engine context became ready");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until the live engine has observed a content-type transition before
/// sending more keys. The property write is synchronous to ibus-daemon, but
/// forwarding it to an engine instance may complete asynchronously.
fn wait_for_content_type(
    status_file: &Path,
    expected: wayexpand_backend_ibus::telemetry::IbusContentTypeState,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(reading) = wayexpand_backend_ibus::telemetry::read_status_at(status_file) {
            if reading
                .status
                .recent_fields
                .iter()
                .any(|field| field.focused && field.content_type == expected)
            {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            bail!("IBus engine did not observe the requested input-field content type");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn main() -> Result<()> {
    let mut arguments = env::args_os().skip(1);
    let status_file: PathBuf = arguments
        .next()
        .map(PathBuf::from)
        .context("usage: wx-e2e-ibus <status-file> [--expect-active]")?;
    let mut expect_active = false;
    let mut single_expansion = false;
    for argument in arguments {
        match argument.to_str() {
            Some("--expect-active") if !expect_active => expect_active = true,
            Some("--single-expansion") if !single_expansion => single_expansion = true,
            _ => bail!("usage: wx-e2e-ibus <status-file> [--expect-active] [--single-expansion]"),
        }
    }
    let address = env::var("IBUS_ADDRESS").context("IBUS_ADDRESS is not set")?;
    let connection = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
    let (sender, events) = mpsc::channel();
    let messages = MessageIterator::from(&connection);
    thread::spawn(move || {
        for message in messages.flatten() {
            let header = message.header();
            let (Some(member), Some(path)) = (header.member(), header.path()) else {
                continue;
            };
            let path = path.to_string();
            let event = match member.as_str() {
                "CommitText" => message
                    .body()
                    .deserialize::<(OwnedValue,)>()
                    .ok()
                    .and_then(|(value,)| text_of(&value))
                    .map(Event::Commit),
                "DeleteSurroundingText" => message
                    .body()
                    .deserialize::<(i32, u32)>()
                    .ok()
                    .map(|(offset, count)| Event::Delete(offset, count)),
                _ => None,
            };
            if let Some(event) = event {
                let _ = sender.send((path, event));
            }
        }
    });

    let focus_baseline = wayexpand_backend_ibus::telemetry::read_status_at(&status_file)
        .map(|reading| reading.status.totals.focus_ins)
        .unwrap_or(0);
    let mut results = Results { failed: false };
    let full_caps = CAP_PREEDIT | CAP_FOCUS | CAP_SURROUNDING_TEXT;
    let mut editor = create_field(&connection, &events, "e2e-editor", full_caps, true)?;
    editor.set_content_type(PURPOSE_FREE_FORM)?;
    match select_engine(&connection, &editor, expect_active) {
        Ok(()) if expect_active => results.check(
            "gnome-selected-engine",
            true,
            "the engine selected by GNOME was already active in IBus",
        ),
        Ok(()) => results.check("registered", true, "SetGlobalEngine selected wayexpand"),
        Err(error) => {
            results.check("registered", false, format!("{error:#}"));
            std::process::exit(1);
        }
    }
    // An IBus global engine descriptor can be selected before the new
    // per-context engine is attached, especially after GNOME Shell restarts.
    // Wait for WayExpand to publish a focused normal-text context before
    // sending even a harmless probe key: older IBus releases may report the
    // key handled while that attachment is still in flight.
    wait_for_focused_engine(&status_file, focus_baseline)?;
    if single_expansion {
        // Check that a non-trigger printable key still reaches the client
        // after the newly selected IBus context is ready.
        editor.type_text("x")?;
        results.check(
            "engine-start-probe",
            editor.contents() == "x",
            format!(
                "harmless probe text remained intact: {:?}",
                editor.contents()
            ),
        );
        if results.failed {
            std::process::exit(1);
        }
        editor.clear()?;
    }
    results.check(
        "engine-context-ready",
        true,
        "IBus reports a focused normal-text context with surrounding-text capability",
    );

    editor.clear()?;
    editor.type_text(TRIGGER)?;
    results.check(
        "expands",
        editor.contents() == REPLACEMENT,
        format!("text after typing the trigger: {:?}", editor.contents()),
    );

    if single_expansion {
        thread::sleep(Duration::from_millis(200));
        match wayexpand_backend_ibus::telemetry::read_status_at(&status_file) {
            Some(reading) => {
                let status = reading.status;
                let raw = std::fs::read_to_string(&status_file).unwrap_or_default();
                results.check(
                    "telemetry-counts",
                    status.totals.expansions >= 1,
                    format!("{} expansion(s) recorded", status.totals.expansions),
                );
                results.check(
                    "telemetry-content-free",
                    !raw.contains(TRIGGER) && !raw.contains(REPLACEMENT) && !raw.contains("e2e"),
                    "status file contains no typed text, trigger, or replacement",
                );
            }
            None => results.check(
                "telemetry-counts",
                false,
                format!("no status at {}", status_file.display()),
            ),
        }
        if results.failed {
            std::process::exit(1);
        }
        return Ok(());
    }

    // Same field type again: ibus-daemon does not set ContentType again.
    editor.call("FocusOut", &())?;
    editor.call("FocusIn", &())?;
    editor.clear()?;
    editor.type_text(TRIGGER)?;
    results.check(
        "refocus-same-type",
        editor.contents() == REPLACEMENT,
        format!("text after refocus: {:?}", editor.contents()),
    );

    editor.set_content_type(PURPOSE_PASSWORD)?;
    wait_for_content_type(
        &status_file,
        wayexpand_backend_ibus::telemetry::IbusContentTypeState::Sensitive,
    )?;
    editor.clear()?;
    editor.type_text(TRIGGER)?;
    results.check(
        "password-field",
        editor.contents() == TRIGGER,
        format!(
            "password field kept the typed text: {:?}",
            editor.contents()
        ),
    );

    editor.set_content_type(PURPOSE_FREE_FORM)?;
    wait_for_content_type(
        &status_file,
        wayexpand_backend_ibus::telemetry::IbusContentTypeState::Normal,
    )?;
    editor.clear()?;
    editor.type_text(TRIGGER)?;
    results.check(
        "back-to-text-field",
        editor.contents() == REPLACEMENT,
        format!(
            "text after leaving the password field: {:?}",
            editor.contents()
        ),
    );
    editor.call("FocusOut", &())?;

    // A terminal-like client: it never reports the text before the cursor.
    let mut terminal = create_field(
        &connection,
        &events,
        "e2e-terminal",
        CAP_PREEDIT | CAP_FOCUS,
        false,
    )?;
    terminal.call("FocusIn", &())?;
    terminal.set_content_type(PURPOSE_FREE_FORM)?;
    terminal.type_text(TRIGGER)?;
    results.check(
        "no-surrounding-text",
        terminal.contents() == TRIGGER,
        format!(
            "nothing deleted without surrounding text: {:?}",
            terminal.contents()
        ),
    );
    terminal.call("FocusOut", &())?;

    // Telemetry: the engine published its activity, passed terminal
    // input through without capturing it, and never recorded typed text.
    thread::sleep(Duration::from_millis(800));
    match wayexpand_backend_ibus::telemetry::read_status_at(&status_file) {
        Some(reading) => {
            let status = reading.status;
            let raw = std::fs::read_to_string(&status_file).unwrap_or_default();
            let field_summary = status
                .recent_fields
                .iter()
                .map(|field| {
                    format!(
                        "focused={} type={:?} surrounding={} keys={} uncaptured={} expansions={} refusals={:?}",
                        field.focused,
                        field.content_type,
                        field.surrounding_capability,
                        field.keys,
                        field.keys_without_capture,
                        field.expansions,
                        field.refusals
                    )
                })
                .collect::<Vec<_>>();
            results.check(
                "telemetry-counts",
                status.totals.expansions >= 3,
                format!(
                    "{} expansion(s) recorded; fields={field_summary:?}",
                    status.totals.expansions
                ),
            );
            let explained = status.recent_fields.iter().any(|field| {
                !field.surrounding_capability
                    && field.keys_without_capture >= TRIGGER.chars().count() as u64
                    && wayexpand_backend_ibus::IbusFieldVerdict::of(field)
                        == wayexpand_backend_ibus::IbusFieldVerdict::NoSurroundingText
            });
            results.check(
                "telemetry-explains-terminal",
                explained,
                "unsupported terminal-like field passed through without capture and reported as no-surrounding-text",
            );
            results.check(
                "telemetry-content-free",
                !raw.contains(TRIGGER) && !raw.contains(REPLACEMENT) && !raw.contains("e2e"),
                "status file contains no typed text, trigger, or replacement",
            );
        }
        None => results.check(
            "telemetry-counts",
            false,
            format!("no status at {}", status_file.display()),
        ),
    }

    if results.failed {
        std::process::exit(1);
    }
    Ok(())
}
