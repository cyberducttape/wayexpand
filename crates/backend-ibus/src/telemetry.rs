//! Content-free activity telemetry for the IBus engine.
//!
//! The engine publishes what it observed about recently focused fields so
//! diagnostics can explain why a trigger did or did not expand without a
//! D-Bus monitor. Only counters and states are recorded: never typed
//! characters, surrounding text, triggers, or replacements.

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    env, fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Version of the status file contract read by `wayexpand doctor`.
pub const IBUS_STATUS_SCHEMA: u32 = 1;
const STATUS_FILE_NAME: &str = "wayexpand-ibus-status.json";
const MAX_RECENT_FIELDS: usize = 8;
const MAX_STATUS_BYTES: u64 = 64 * 1024;
static TRANSACTION_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What the focused client last reported about the field's type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IbusContentTypeState {
    /// IBus has not set a content type for this engine; capture stays off.
    #[default]
    Unknown,
    Normal,
    Sensitive,
}

/// Why a matched trigger was not replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IbusRefusal {
    /// The client does not advertise `IBUS_CAP_SURROUNDING_TEXT`.
    NoSurroundingCapability,
    /// The client has not reported its text since the last change.
    SurroundingTextUnknown,
    /// The client reports empty text even after typing (typical of terminals).
    SurroundingTextEmpty,
    /// The reported text does not end with the trigger, or has a selection.
    SurroundingTextMismatch,
    /// Organization policy or the compatibility setting refused the edit.
    Policy,
}

/// Counters for one refusal reason each, so the JSON stays flat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IbusRefusalCounts {
    pub no_surrounding_capability: u64,
    pub surrounding_text_unknown: u64,
    pub surrounding_text_empty: u64,
    pub surrounding_text_mismatch: u64,
    pub policy: u64,
}

impl IbusRefusalCounts {
    fn add(&mut self, refusal: IbusRefusal) {
        let counter = match refusal {
            IbusRefusal::NoSurroundingCapability => &mut self.no_surrounding_capability,
            IbusRefusal::SurroundingTextUnknown => &mut self.surrounding_text_unknown,
            IbusRefusal::SurroundingTextEmpty => &mut self.surrounding_text_empty,
            IbusRefusal::SurroundingTextMismatch => &mut self.surrounding_text_mismatch,
            IbusRefusal::Policy => &mut self.policy,
        };
        *counter = counter.saturating_add(1);
    }

    pub fn total(&self) -> u64 {
        self.no_surrounding_capability
            .saturating_add(self.surrounding_text_unknown)
            .saturating_add(self.surrounding_text_empty)
            .saturating_add(self.surrounding_text_mismatch)
            .saturating_add(self.policy)
    }
}

/// Activity observed while one field had focus.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IbusFieldActivity {
    /// Increases by one for every focus-in this engine process has seen.
    pub sequence: u64,
    pub focused: bool,
    pub started_unix: u64,
    pub content_type: IbusContentTypeState,
    pub surrounding_capability: bool,
    pub surrounding_reports: u64,
    pub surrounding_nonempty: bool,
    pub keys: u64,
    /// Keys passed through untouched because capture was off or sensitive.
    pub keys_without_capture: u64,
    pub expansions: u64,
    pub refusals: IbusRefusalCounts,
    pub last_refusal: Option<IbusRefusal>,
    pub output_failures: u64,
}

/// Totals since the engine process started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IbusTotals {
    pub focus_ins: u64,
    pub keys: u64,
    pub expansions: u64,
    pub refusals: IbusRefusalCounts,
    pub output_failures: u64,
}

/// The published status file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IbusStatus {
    pub schema: u32,
    pub pid: u32,
    pub version: String,
    pub started_unix: u64,
    pub updated_unix: u64,
    /// Engine objects IBus currently holds.
    pub engines: u64,
    pub totals: IbusTotals,
    /// Most recent last.
    pub recent_fields: Vec<IbusFieldActivity>,
    /// Opaque ID for the most recently emitted expansion, unrelated to text.
    #[serde(default)]
    pub last_transaction_id: Option<String>,
}

#[derive(Debug)]
struct State {
    status: IbusStatus,
    fields: VecDeque<IbusFieldActivity>,
    dirty: bool,
}

/// Shared recorder for every engine object in one engine process.
#[derive(Debug)]
pub struct IbusTelemetry {
    state: Mutex<State>,
    changed: Condvar,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

impl Default for IbusTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl IbusTelemetry {
    pub fn new() -> Self {
        let now = now_unix();
        Self {
            state: Mutex::new(State {
                status: IbusStatus {
                    schema: IBUS_STATUS_SCHEMA,
                    pid: std::process::id(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    started_unix: now,
                    updated_unix: now,
                    engines: 0,
                    totals: IbusTotals::default(),
                    recent_fields: Vec::new(),
                    last_transaction_id: None,
                },
                fields: VecDeque::new(),
                dirty: true,
            }),
            changed: Condvar::new(),
        }
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        change(&mut state);
        state.dirty = true;
        drop(state);
        self.changed.notify_one();
    }

    fn update_field(&self, sequence: Option<u64>, change: impl FnOnce(&mut IbusFieldActivity)) {
        let Some(sequence) = sequence else {
            return;
        };
        self.update(|state| {
            if let Some(field) = state
                .fields
                .iter_mut()
                .rev()
                .find(|field| field.sequence == sequence)
            {
                change(field);
            }
        });
    }

    pub fn set_engines(&self, engines: usize) {
        self.update(|state| state.status.engines = engines as u64);
    }

    /// Start a field record and return its sequence number.
    pub fn focus_in(&self, previous: Option<u64>) -> u64 {
        let mut sequence = 0;
        self.update(|state| {
            if let Some(previous) = previous {
                if let Some(field) = state.fields.iter_mut().find(|f| f.sequence == previous) {
                    field.focused = false;
                }
            }
            state.status.totals.focus_ins = state.status.totals.focus_ins.saturating_add(1);
            sequence = state.status.totals.focus_ins;
            if state.fields.len() == MAX_RECENT_FIELDS {
                state.fields.pop_front();
            }
            state.fields.push_back(IbusFieldActivity {
                sequence,
                focused: true,
                started_unix: now_unix(),
                ..IbusFieldActivity::default()
            });
        });
        sequence
    }

    pub fn focus_out(&self, field: Option<u64>) {
        self.update_field(field, |field| field.focused = false);
    }

    pub fn capabilities(&self, field: Option<u64>, surrounding: bool) {
        self.update_field(field, |field| field.surrounding_capability = surrounding);
    }

    pub fn content_type(&self, field: Option<u64>, sensitive: bool) {
        self.update_field(field, |field| {
            field.content_type = if sensitive {
                IbusContentTypeState::Sensitive
            } else {
                IbusContentTypeState::Normal
            };
        });
    }

    pub fn surrounding_text(&self, field: Option<u64>, nonempty: bool) {
        self.update_field(field, |field| {
            field.surrounding_reports = field.surrounding_reports.saturating_add(1);
            field.surrounding_nonempty |= nonempty;
        });
    }

    pub fn key(&self, field: Option<u64>, captured: bool) {
        self.update(|state| state.status.totals.keys = state.status.totals.keys.saturating_add(1));
        self.update_field(field, |field| {
            field.keys = field.keys.saturating_add(1);
            if !captured {
                field.keys_without_capture = field.keys_without_capture.saturating_add(1);
            }
        });
    }

    pub fn expansion(&self, field: Option<u64>) {
        let transaction_id = format!(
            "{:x}-{:x}",
            std::process::id(),
            TRANSACTION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.update(|state| {
            state.status.totals.expansions = state.status.totals.expansions.saturating_add(1);
            state.status.last_transaction_id = Some(transaction_id);
        });
        self.update_field(field, |field| {
            field.expansions = field.expansions.saturating_add(1)
        });
    }

    pub fn refusal(&self, field: Option<u64>, refusal: IbusRefusal) {
        self.update(|state| state.status.totals.refusals.add(refusal));
        self.update_field(field, |field| {
            field.refusals.add(refusal);
            field.last_refusal = Some(refusal);
        });
    }

    pub fn output_failure(&self, field: Option<u64>) {
        self.update(|state| {
            state.status.totals.output_failures =
                state.status.totals.output_failures.saturating_add(1)
        });
        self.update_field(field, |field| {
            field.output_failures = field.output_failures.saturating_add(1)
        });
    }

    /// A copy of the current status.
    pub fn snapshot(&self) -> IbusStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self::snapshot_of(&state)
    }

    fn snapshot_of(state: &State) -> IbusStatus {
        let mut status = state.status.clone();
        status.updated_unix = now_unix();
        status.recent_fields = state.fields.iter().cloned().collect();
        status
    }

    /// Block until something changed (or `timeout` passes), then return a
    /// snapshot if there is anything new to publish.
    pub fn wait_for_change(&self, timeout: Duration) -> Option<IbusStatus> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| !state.dirty)
            .unwrap_or_else(|poison| poison.into_inner());
        if !state.dirty {
            return None;
        }
        state.dirty = false;
        Some(Self::snapshot_of(&state))
    }
}

/// Where the engine publishes its status: the user's private runtime
/// directory. `None` without a usable `XDG_RUNTIME_DIR`.
pub fn status_path() -> Option<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())?;
    let runtime = PathBuf::from(runtime);
    runtime
        .is_absolute()
        .then(|| runtime.join(STATUS_FILE_NAME))
}

/// Only publish into a directory that belongs to this user and that no one
/// else can write, so the file cannot be redirected or replaced.
fn private_directory(directory: &Path) -> bool {
    fs::symlink_metadata(directory).is_ok_and(|metadata| {
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o022 == 0
    })
}

/// Atomically replace the status file with `status` (mode 0600).
pub fn write_status(path: &Path, status: &IbusStatus) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::other("status path has no parent"))?;
    if !private_directory(directory) {
        return Err(std::io::Error::other(
            "runtime directory is not private to this user",
        ));
    }
    let body = serde_json::to_vec(status).map_err(std::io::Error::other)?;
    let temporary = directory.join(format!(".{STATUS_FILE_NAME}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// A status file read back by diagnostics.
#[derive(Debug, Clone)]
pub struct IbusStatusReading {
    pub status: IbusStatus,
    /// The publishing engine process is still running.
    pub live: bool,
}

/// Read the engine's published status. Files that are not private, too
/// large, from another schema, or unparsable are ignored.
pub fn read_status() -> Option<IbusStatusReading> {
    read_status_at(&status_path()?)
}

pub fn read_status_at(path: &Path) -> Option<IbusStatusReading> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > MAX_STATUS_BYTES
    {
        return None;
    }
    let status: IbusStatus = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if status.schema != IBUS_STATUS_SCHEMA {
        return None;
    }
    let live = fs::read_to_string(format!("/proc/{}/comm", status.pid))
        .is_ok_and(|comm| comm.trim() == "wayexpand-ibus");
    Some(IbusStatusReading { status, live })
}

/// A plain-language verdict about one observed field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbusFieldVerdict {
    /// At least one expansion was emitted in this field.
    Expanded,
    /// The field supports verified replacement; no trigger was typed yet.
    Ready,
    /// IBus never told the engine the field's type, so capture stayed off.
    NoContentType,
    /// The field is a password, PIN, or private field; capture stays off.
    Sensitive,
    /// The client cannot show the engine its text (terminals, some
    /// Electron/XWayland clients); IBus cannot replace text here.
    NoSurroundingText,
    /// A trigger matched but the client's text did not confirm it.
    Mismatch,
    /// A trigger matched but policy or the compatibility setting refused.
    Policy,
    /// The engine saw no keys in this field.
    Idle,
}

impl IbusFieldVerdict {
    pub fn of(field: &IbusFieldActivity) -> Self {
        if field.expansions > 0 {
            return Self::Expanded;
        }
        match field.content_type {
            IbusContentTypeState::Sensitive => return Self::Sensitive,
            IbusContentTypeState::Unknown if field.keys > 0 => return Self::NoContentType,
            _ => {}
        }
        match field.last_refusal {
            Some(IbusRefusal::Policy) => return Self::Policy,
            Some(IbusRefusal::SurroundingTextMismatch) if field.surrounding_nonempty => {
                return Self::Mismatch
            }
            Some(_) => return Self::NoSurroundingText,
            None => {}
        }
        if field.keys == 0 {
            return Self::Idle;
        }
        // Typing without a matched trigger: judge readiness from what the
        // client reported, so an app can be checked before a trigger is typed.
        if !field.surrounding_capability || !field.surrounding_nonempty {
            Self::NoSurroundingText
        } else {
            Self::Ready
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Ready => "ready",
            Self::NoContentType => "no-content-type",
            Self::Sensitive => "sensitive",
            Self::NoSurroundingText => "no-surrounding-text",
            Self::Mismatch => "text-mismatch",
            Self::Policy => "refused-by-policy",
            Self::Idle => "idle",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::Expanded => "expansions work in this field",
            Self::Ready => {
                "the app reports its field type and text; triggers typed here can expand"
            }
            Self::NoContentType => {
                "IBus never reported this field's type, so capture stayed off (restart the engine: `ibus engine wayexpand`; if it persists, report a bug with `wayexpand support-bundle`)"
            }
            Self::Sensitive => "password, PIN, or private field: expansion is intentionally off",
            Self::NoSurroundingText => {
                "the app does not show IBus the text before the cursor (typical of terminals), so IBus cannot replace a trigger here; list the app in `terminal_apps` and use `wayexpand setup --mode terminals`"
            }
            Self::Mismatch => {
                "the app's reported text did not end with the trigger (cursor moved, selection, or autocorrect), so the replacement was refused"
            }
            Self::Policy => {
                "organization policy or `compatibility_mode` refused the replacement; see `wayexpand doctor --security`"
            }
            Self::Idle => "no keys were seen in this field",
        }
    }
}

/// Classify why surrounding text failed to confirm a trigger: the client
/// lacks the capability, never reported any text for this field (terminals
/// report nothing or an empty string), has not reported since the last
/// change, or reported text that does not end with the trigger.
pub(crate) fn classify_surrounding_refusal(
    capability: bool,
    text_seen: bool,
    model_present: bool,
) -> IbusRefusal {
    if !capability {
        IbusRefusal::NoSurroundingCapability
    } else if !text_seen {
        IbusRefusal::SurroundingTextEmpty
    } else if !model_present {
        IbusRefusal::SurroundingTextUnknown
    } else {
        IbusRefusal::SurroundingTextMismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_records_are_bounded_and_ordered() {
        let telemetry = IbusTelemetry::new();
        let mut previous = None;
        for _ in 0..(MAX_RECENT_FIELDS + 3) {
            previous = Some(telemetry.focus_in(previous));
        }
        let status = telemetry.snapshot();
        assert_eq!(status.recent_fields.len(), MAX_RECENT_FIELDS);
        assert_eq!(
            status.recent_fields.last().unwrap().sequence,
            (MAX_RECENT_FIELDS + 3) as u64
        );
        assert_eq!(status.recent_fields.iter().filter(|f| f.focused).count(), 1);
    }

    #[test]
    fn expansion_telemetry_exposes_only_an_opaque_transaction_id() {
        let telemetry = IbusTelemetry::new();
        let field = Some(telemetry.focus_in(None));
        telemetry.expansion(field);
        let status = telemetry.snapshot();
        let id = status
            .last_transaction_id
            .expect("emitted expansion has an ID");
        assert!(id.contains('-'));
        assert_eq!(status.totals.expansions, 1);
        assert!(!id.contains("trigger"));
        assert!(!id.contains("replacement"));
    }

    #[test]
    fn verdicts_explain_terminal_like_and_sensitive_fields() {
        let telemetry = IbusTelemetry::new();
        let field = Some(telemetry.focus_in(None));
        telemetry.capabilities(field, true);
        telemetry.content_type(field, false);
        telemetry.surrounding_text(field, false);
        for _ in 0..3 {
            telemetry.key(field, true);
        }
        telemetry.refusal(field, IbusRefusal::SurroundingTextEmpty);
        let terminal = telemetry.snapshot().recent_fields.pop().unwrap();
        assert_eq!(
            IbusFieldVerdict::of(&terminal),
            IbusFieldVerdict::NoSurroundingText
        );

        let field = Some(telemetry.focus_in(field));
        telemetry.content_type(field, true);
        telemetry.key(field, false);
        let password = telemetry.snapshot().recent_fields.pop().unwrap();
        assert_eq!(IbusFieldVerdict::of(&password), IbusFieldVerdict::Sensitive);

        let field = Some(telemetry.focus_in(field));
        telemetry.capabilities(field, true);
        telemetry.content_type(field, false);
        telemetry.surrounding_text(field, true);
        telemetry.key(field, true);
        let editor = telemetry.snapshot().recent_fields.pop().unwrap();
        assert_eq!(IbusFieldVerdict::of(&editor), IbusFieldVerdict::Ready);
        telemetry.expansion(field);
        let editor = telemetry.snapshot().recent_fields.pop().unwrap();
        assert_eq!(IbusFieldVerdict::of(&editor), IbusFieldVerdict::Expanded);
    }

    #[test]
    fn status_round_trips_through_a_private_file_only() {
        let directory =
            std::env::temp_dir().join(format!("wayexpand-ibus-status-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join(STATUS_FILE_NAME);
        let telemetry = IbusTelemetry::new();
        telemetry.focus_in(None);
        write_status(&path, &telemetry.snapshot()).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let reading = read_status_at(&path).unwrap();
        assert_eq!(reading.status.recent_fields.len(), 1);
        // A group- or world-readable file is not trusted.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_status_at(&path).is_none());
        // Neither is a shared directory.
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(write_status(&path, &telemetry.snapshot()).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn status_never_serializes_text() {
        let status = IbusTelemetry::new().snapshot();
        let json = serde_json::to_value(&status).unwrap();
        let field = serde_json::to_value(IbusFieldActivity::default()).unwrap();
        let keys: Vec<&str> = field
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(keys
            .iter()
            .all(|key| !matches!(*key, "text" | "trigger" | "replacement")));
        assert!(json.get("recent_fields").is_some());
    }
}
