//! Protocol-neutral IBus engine adapter.
//!
//! IBus owns the key event loop and asks an engine whether each key was
//! handled.  This adapter deliberately contains no D-Bus code: a small host
//! (the native IBus service or a test harness) translates [`IbusAction`]s to
//! IBus signals. Keeping that boundary explicit makes the expansion behavior
//! testable and prevents D-Bus threading details from entering the matcher.

#[cfg(test)]
use std::{
    thread,
    time::{Duration, Instant},
};

use std::sync::Arc;
use tracing::warn;
use wayexpand_core::{
    CompletionNotifier, Config, ExpansionEngine, ExpansionResult, InjectorCapabilities, InputEvent,
    InputSourceCapabilities, OrganizationPolicy, PendingExpansionDispatch, ReplacementAdmission,
    ReplacementGuarantee, ValidatedConfig, WindowContext, MAX_WINDOW_INSTANCE_ID_BYTES,
};

// IBus' public C API defines IBUS_RELEASE_MASK as (1 << 30). The ibus-rs
// crate is not used because it adds a mandatory libdbus system dependency.
const IBUS_RELEASE_MASK: u32 = 1 << 30;

/// Organization-policy identity of this backend. IBus is governed separately
/// from the native input-method-v2 route: their surrounding-text, sensitive-
/// field, and replacement guarantees differ, so `allowed_backends` must name
/// `ibus` explicitly to permit it.
pub const IBUS_BACKEND_NAME: &str = "ibus";
const IBUS_CONTEXT_ID_PREFIX: &str = "ibus-context:";

#[cfg(test)]
pub(crate) static FORM_HELPER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `IBUS_CAP_SURROUNDING_TEXT`: the client can report surrounding text and
/// honour `DeleteSurroundingText`.
pub const IBUS_CAP_SURROUNDING_TEXT: u32 = 1 << 5;

// IBusInputPurpose / IBusInputHints values from ibustypes.h.
const IBUS_INPUT_PURPOSE_PASSWORD: u32 = 8;
const IBUS_INPUT_PURPOSE_PIN: u32 = 9;
const IBUS_INPUT_PURPOSE_LAST_KNOWN: u32 = 13; // IBUS_INPUT_PURPOSE_DATETIME
const IBUS_INPUT_HINT_PRIVATE: u32 = 1 << 11;
const IBUS_INPUT_HINT_HIDDEN_TEXT: u32 = 1 << 12;

mod availability;
#[cfg(test)]
use availability::component_file_present_in;
pub use availability::{engine_available, engine_installed, global_engine_selected};
pub mod chain;
pub mod gnome_input_sources;
pub mod telemetry;
pub use telemetry::{IbusFieldVerdict, IbusRefusal, IbusTelemetry};
mod text;
use text::{keysym_to_char, replacement_actions, SurroundingText};
mod policy;

/// Whether an IBus content type must disable capture. Password and PIN
/// purposes, the private and hidden-text hints, and any purpose newer than
/// this build knows are all treated as sensitive: unknown means sensitive.
pub fn content_type_is_sensitive(purpose: u32, hints: u32) -> bool {
    matches!(
        purpose,
        IBUS_INPUT_PURPOSE_PASSWORD | IBUS_INPUT_PURPOSE_PIN
    ) || purpose > IBUS_INPUT_PURPOSE_LAST_KNOWN
        || hints & (IBUS_INPUT_HINT_PRIVATE | IBUS_INPUT_HINT_HIDDEN_TEXT) != 0
}

/// Output guarantees of the IBus route. Delete and commit are two separate
/// D-Bus signals with an observable failure boundary between them, so the
/// replacement is not atomic.
pub fn injector_capabilities() -> InjectorCapabilities {
    InjectorCapabilities {
        insertion_mode: "ibus commit-text",
        max_text_chars: 0,
        expected_throughput_chars_per_sec: None,
        atomic_replace: false,
        // The engine refuses unless IBus surrounding text confirms the
        // trigger before the cursor.
        replacement_guarantee: wayexpand_core::ReplacementGuarantee::VerifiedSurroundingText,
        full_unicode: true,
        cursor_reposition: false,
        key_passthrough: true,
    }
}

/// Capture guarantees of the IBus route. Sensitive-field focus is reported
/// through IBus content types and fails closed until one is received.
pub fn source_capabilities() -> InputSourceCapabilities {
    InputSourceCapabilities {
        sensitive_focus: true,
        exclusive_capture: true,
        reliable_key_state: false,
        key_passthrough: true,
        composition_aware: false,
        local_compose_aware: false,
        layout_aware: true,
    }
}

mod service;

pub use service::{run_service, IbusServiceError};

/// Output operations an IBus host must emit on the engine's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IbusAction {
    /// Delete `nchars` Unicode scalar values before the insertion cursor.
    DeleteSurroundingText { nchars: u32 },
    /// Commit text at the application cursor.
    CommitText(wayexpand_core::SensitiveText),
}

/// Result of processing one IBus key press.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IbusKeyResult {
    /// Whether the engine consumed the key. If false, the IBus host must let
    /// the client process the original key event.
    pub handled: bool,
    pub actions: Vec<IbusAction>,
}

/// Core-backed IBus engine state.
pub struct IbusEngineAdapter {
    engine: ExpansionEngine,
    enabled: bool,
    output_retired: bool,
    policy: OrganizationPolicy,
    client_capabilities: u32,
    surrounding: Option<SurroundingText>,
    /// The `(purpose, hints)` IBus last set on this engine object, if any.
    /// ibus-daemon caches the same value per engine proxy and only sets the
    /// property again when a focused context's type differs, so this is the
    /// content type of a refocused field until a new one arrives.
    content_type: Option<(u32, u32)>,
    /// Content-free activity recorder shared by the engine process.
    telemetry: Option<Arc<IbusTelemetry>>,
    /// The telemetry record of the field this engine currently serves.
    field: Option<u64>,
    /// The client reported non-empty text at least once for this field.
    field_text_seen: bool,
    /// Set when safe-mode policy requires guarantees this route lacks
    /// (for example `require_atomic_replace`). Expansion is then disabled.
    capability_block: Option<String>,
    pending_output: Option<ExpansionResult>,
    last_expansion_outcome: Option<IbusExpansionOutcome>,
    /// GNOME window identity observed when this IBus input context received
    /// focus. App filters are enabled only while the authenticated Shell
    /// tracker continues to report that same focused window.
    gnome_focus_binding: Option<String>,
    ibus_context_identity: Option<String>,
}

/// An emitted IBus signal sequence is not confirmation that the target
/// application applied the edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbusExpansionOutcome {
    EmittedUnverified,
    /// Output failed after one or more actions may have reached the client.
    UnknownPartialEmission,
}

impl IbusEngineAdapter {
    pub fn new(engine: ExpansionEngine) -> Self {
        Self::build(engine, OrganizationPolicy::default())
    }

    pub fn with_policy(
        mut engine: ExpansionEngine,
        policy: OrganizationPolicy,
    ) -> Result<Self, wayexpand_core::ConfigError> {
        engine.apply_administrator_policy(&policy)?;
        Ok(Self::build(engine, policy))
    }

    /// Build an independent input-context adapter over the same immutable
    /// validated generation. The matcher's trie and compiled configuration
    /// metadata are shared; only the engine's mutable typing state is new.
    pub fn with_validated_config(
        validated: Arc<ValidatedConfig>,
        policy: OrganizationPolicy,
    ) -> Self {
        Self::build(ExpansionEngine::from_validated(validated), policy)
    }

    fn build(mut engine: ExpansionEngine, policy: OrganizationPolicy) -> Self {
        policy::apply_to_engine(&mut engine, &policy);
        // IBus checks the surrounding text before deleting a trigger. This
        // is verified (though not atomic) replacement, which is sufficient
        // for opening a form only when the IBus input-context identity is
        // also available.
        engine.set_form_replacement_guarantee(ReplacementGuarantee::VerifiedSurroundingText);
        // Text-only input contexts need no command workers. Keep IBus key
        // handling non-blocking only for configurations that actually enable
        // command expansions or hotkeys; direct programs are still rejected
        // by the backend-specific gate below.
        if engine.requires_async_workers() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not start; command-backed actions are unavailable"
            );
        }
        // A new engine has no content type yet: capture stays off until the
        // client reports a known non-sensitive field.
        engine.process(InputEvent::FocusChanged { sensitive: true });
        let capability_block = Self::check_capabilities(&policy);
        Self {
            engine,
            enabled: true,
            output_retired: false,
            policy,
            client_capabilities: 0,
            surrounding: None,
            content_type: None,
            telemetry: None,
            field: None,
            field_text_seen: false,
            capability_block,
            pending_output: None,
            last_expansion_outcome: None,
            gnome_focus_binding: None,
            ibus_context_identity: None,
        }
    }

    /// Verify policy requirements against the real IBus capability profile,
    /// as the daemon does for its negotiated source and injector.
    fn check_capabilities(policy: &OrganizationPolicy) -> Option<String> {
        policy::capability_block(policy, injector_capabilities(), source_capabilities())
    }

    /// Why safe-mode policy has disabled expansion for this route, if it has.
    pub fn capability_block(&self) -> Option<&str> {
        self.capability_block.as_deref()
    }

    /// Register a wakeup for asynchronous command completions; see
    /// [`ExpansionEngine::set_completion_notifier`].
    pub fn set_completion_notifier(&mut self, notifier: Option<CompletionNotifier>) {
        self.engine.set_completion_notifier(notifier);
    }

    /// Record this engine's activity (content-free) in `telemetry`.
    pub fn set_telemetry(&mut self, telemetry: Arc<IbusTelemetry>) {
        self.telemetry = Some(telemetry);
    }

    fn note(&self, record: impl FnOnce(&IbusTelemetry, Option<u64>)) {
        if let Some(telemetry) = &self.telemetry {
            record(telemetry, self.field);
        }
    }

    /// Record why surrounding text did not confirm a matched trigger.
    fn note_surrounding_refusal(&self) {
        let refusal = telemetry::classify_surrounding_refusal(
            self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0,
            self.field_text_seen,
            self.surrounding.is_some(),
        );
        self.note(|telemetry, field| telemetry.refusal(field, refusal));
    }

    fn advertises_surrounding_text(&self) -> bool {
        self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0
    }

    /// Apply the IBus `ContentType` property. A content type IBus set is the
    /// only way capture is enabled for a field.
    pub fn set_content_type(&mut self, purpose: u32, hints: u32) {
        self.content_type = Some((purpose, hints));
        self.apply_content_type();
    }

    /// Re-derive field sensitivity from the last content type IBus set.
    /// Without one, capture stays off.
    fn apply_content_type(&mut self) {
        let sensitive = self
            .content_type
            .is_none_or(|(purpose, hints)| content_type_is_sensitive(purpose, hints));
        if self.content_type.is_some() {
            self.note(|telemetry, field| telemetry.content_type(field, sensitive));
        }
        self.engine.process(InputEvent::FocusChanged { sensitive });
    }

    /// Apply an IBus `SetCapabilities`.
    pub fn set_capabilities(&mut self, capabilities: u32) {
        self.client_capabilities = capabilities;
        if capabilities & IBUS_CAP_SURROUNDING_TEXT == 0 {
            self.surrounding = None;
        }
        self.note(|telemetry, field| {
            telemetry.capabilities(field, capabilities & IBUS_CAP_SURROUNDING_TEXT != 0)
        });
    }

    /// Apply an IBus `SetSurroundingText`. `cursor` and `anchor` are in
    /// Unicode scalar values.
    pub fn set_surrounding_text(&mut self, text: &str, cursor: u32, anchor: u32) {
        self.field_text_seen |= !text.is_empty();
        self.note(|telemetry, field| telemetry.surrounding_text(field, !text.is_empty()));
        let text: Vec<char> = text.chars().collect();
        let (cursor, anchor) = (cursor as usize, anchor as usize);
        self.surrounding = (self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0
            && cursor <= text.len()
            && anchor <= text.len())
        .then_some(SurroundingText::new(text, cursor, anchor));
    }

    /// Forget the surrounding-text model; replacements are refused until the
    /// client reports its text again.
    pub fn clear_surrounding_text(&mut self) {
        self.surrounding = None;
    }

    /// Whether the client's reported text agrees that `delivered` sits
    /// immediately before a collapsed cursor. Nothing destructive may be
    /// emitted unless it does.
    fn surrounding_confirms(&self, delivered: &str) -> bool {
        self.client_capabilities & IBUS_CAP_SURROUNDING_TEXT != 0
            && self
                .surrounding
                .as_ref()
                .is_some_and(|surrounding| surrounding.ends_with_at_cursor(delivered))
    }

    /// Advance the local surrounding-text model over emitted actions.
    fn record_emitted(&mut self, actions: &[IbusAction]) {
        if let Some(surrounding) = self.surrounding.as_mut() {
            if !actions.iter().all(|action| surrounding.apply(action)) {
                self.surrounding = None;
            }
        }
    }

    pub fn engine(&self) -> &ExpansionEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut ExpansionEngine {
        &mut self.engine
    }

    /// Test-only sink that treats collecting actions as successful emission.
    /// Production hosts must use [`Self::drain_completed_commands_with`] so
    /// accounting is committed only after the output layer acknowledges them.
    #[cfg(test)]
    fn drain_completed_commands_for_test(&mut self) -> Vec<IbusAction> {
        let mut actions = Vec::new();
        let _ = self.drain_completed_commands_with(|batch| {
            actions.extend_from_slice(batch);
            Ok::<(), std::convert::Infallible>(())
        });
        actions
    }

    /// Drain completed asynchronous commands, emitting each replacement
    /// through `emit` and recording it as applied only after `emit`
    /// succeeds. On an emit failure the engine is reset (the client may hold
    /// a partial edit) and the error is returned.
    pub fn drain_completed_commands_with<E>(
        &mut self,
        mut emit: impl FnMut(&[IbusAction]) -> Result<(), E>,
    ) -> Result<usize, E> {
        let mut emitted = 0;
        for result in self.engine.drain_completed_commands() {
            if !self.enabled
                || self.capability_block.is_some()
                || injector_capabilities().replacement_admission(self.engine.compatibility_mode())
                    == ReplacementAdmission::Refused
            {
                self.engine.restore_deferred_match(&result.matched_text);
                continue;
            }
            let violation = self.policy.expansion_policy_violation(
                result.insert.len(),
                result.command_backed,
                IBUS_BACKEND_NAME,
            );
            if policy::blocks(&self.policy, violation, "completed expansion") {
                self.engine.restore_deferred_match(&result.matched_text);
                continue;
            }
            // The key that queued the command was committed at queue time,
            // so the whole trigger (and any delimiter) is in the document.
            let (delivered, actions) = replacement_actions(&result, true);
            if !self.surrounding_confirms(delivered.as_str()) {
                self.note_surrounding_refusal();
                warn!("IBus surrounding text no longer matches the trigger; replacement discarded");
                self.engine.restore_deferred_match(&result.matched_text);
                self.reset();
                continue;
            }
            if let Err(error) = emit(&actions) {
                self.last_expansion_outcome = Some(IbusExpansionOutcome::UnknownPartialEmission);
                self.output_failed();
                return Err(error);
            }
            self.engine.commit_applied_expansion(&result);
            self.last_expansion_outcome = Some(IbusExpansionOutcome::EmittedUnverified);
            self.record_emitted(&actions);
            self.note(|telemetry, field| telemetry.expansion(field));
            emitted += 1;
        }
        Ok(emitted)
    }

    pub fn replace_config(&mut self, config: Config) -> Result<(), wayexpand_core::ConfigError> {
        let mut engine = ExpansionEngine::new(config)?;
        engine.apply_administrator_policy(&self.policy)?;
        engine.inherit_runtime_state(&self.engine);
        if engine.requires_async_workers() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not restart after configuration reload; command-backed actions are unavailable"
            );
        }
        policy::apply_to_engine(&mut engine, &self.policy);
        self.engine = engine;
        Ok(())
    }

    /// Apply a generation already validated and compiled by the IBus service.
    /// This avoids revalidating policy and rebuilding the trigger trie once
    /// for every input context during a reload.
    pub fn replace_validated_config(&mut self, validated: Arc<ValidatedConfig>) {
        let mut engine = ExpansionEngine::from_validated(validated);
        policy::apply_to_engine(&mut engine, &self.policy);
        engine.inherit_runtime_state(&self.engine);
        engine.set_form_replacement_guarantee(ReplacementGuarantee::VerifiedSurroundingText);
        if engine.requires_async_workers() && !engine.enable_async_commands() {
            warn!(
                "IBus asynchronous workers could not restart after configuration reload; command-backed actions are unavailable"
            );
        }
        self.engine = engine;
    }

    /// A context gained focus. This is the only lifecycle event that clears
    /// output retirement after an uncertain partial emission. ibus-daemon
    /// follows FocusIn with SetCapabilities and sets the `ContentType`
    /// property only when the field's type differs from the one it last set
    /// on this engine. Focus therefore clears matcher state and then restores
    /// the remembered content type; with none, capture stays off.
    pub fn focus_in(&mut self) {
        self.focus_in_for_context(None);
    }

    /// A focused IBus engine object belongs to one input context. Retaining
    /// that object's path lets a form result return only to the same text
    /// context after the helper window closes. IBus documents the focus ID as
    /// unique per input context; object paths are generated once per engine
    /// instance by the service factory.
    pub fn focus_in_for_context(&mut self, input_context_path: Option<&str>) {
        self.output_retired = false;
        self.enabled = true;
        self.surrounding = None;
        self.field_text_seen = false;
        if let Some(telemetry) = &self.telemetry {
            self.field = Some(telemetry.focus_in(self.field));
        }
        let identity = input_context_path
            .filter(|identity| {
                !identity.is_empty()
                    && identity.len() + IBUS_CONTEXT_ID_PREFIX.len() <= MAX_WINDOW_INSTANCE_ID_BYTES
            })
            .map(|identity| WindowContext {
                app_id: None,
                title: None,
                instance_id: Some(format!("{IBUS_CONTEXT_ID_PREFIX}{identity}")),
            });
        self.ibus_context_identity = identity
            .as_ref()
            .and_then(|window| window.instance_id.clone());
        self.gnome_focus_binding = None;
        self.engine.process(InputEvent::WindowChanged(identity));
        self.engine
            .process(InputEvent::FocusChanged { sensitive: true });
        self.apply_content_type();
    }

    pub fn focus_out(&mut self) {
        self.note(|telemetry, field| telemetry.focus_out(field));
        self.enabled = false;
        self.surrounding = None;
        self.gnome_focus_binding = None;
        self.ibus_context_identity = None;
        self.engine.process(InputEvent::WindowChanged(None));
        self.engine
            .process(InputEvent::FocusChanged { sensitive: true });
    }

    /// Bind this active IBus context to the authenticated GNOME window that
    /// was focused at FocusIn. `None` deliberately leaves app filtering
    /// unavailable. The Shell identity is revalidated before each key event.
    pub(crate) fn bind_gnome_focus(&mut self, window: Option<&WindowContext>) {
        self.gnome_focus_binding = window
            .and_then(|window| window.instance_id.clone())
            .filter(|identity| !identity.is_empty());
        self.update_gnome_focus(window);
    }

    /// Refresh GNOME app identity against the window bound at FocusIn. A
    /// changed window, missing snapshot, or disconnected tracker clears the
    /// app ID so app-filtered snippets fail closed.
    pub(crate) fn update_gnome_focus(&mut self, window: Option<&WindowContext>) {
        let app_id = match (self.gnome_focus_binding.as_deref(), window) {
            (Some(bound), Some(current)) if current.instance_id.as_deref() == Some(bound) => {
                current.app_id.clone()
            }
            _ => {
                // Once focus diverges or becomes unobservable, this input
                // context must receive a new FocusIn before identity can be
                // trusted again, even if the previous window later returns.
                self.gnome_focus_binding = None;
                None
            }
        };
        let context = self
            .ibus_context_identity
            .as_ref()
            .map(|identity| WindowContext {
                app_id,
                title: None,
                instance_id: Some(identity.clone()),
            });
        self.engine.process(InputEvent::WindowChanged(context));
    }

    /// Apply IBus `Enable` without treating it as proof that focus changed.
    /// In particular, it cannot reactivate an engine retired after uncertain
    /// output; only [`Self::focus_in`] clears that latch.
    pub fn enable(&mut self) {
        if !self.output_retired {
            self.enabled = true;
        }
    }

    /// IBus `Reset`: the cursor may have moved, so the trigger buffer and the
    /// surrounding-text model are dropped. The field's content type is
    /// unchanged by a reset and is kept.
    pub fn reset(&mut self) {
        self.surrounding = None;
        self.engine.process(InputEvent::Reset);
    }

    /// Retire this engine instance after output could have been partially
    /// applied. IBus emits deletion and committed text as separate signals,
    /// so an emission error cannot prove the client document is unchanged.
    /// A fresh focus lifecycle is required before matching resumes.
    pub fn output_failed(&mut self) {
        self.note(|telemetry, field| telemetry.output_failure(field));
        if self.pending_output.take().is_some() {
            self.last_expansion_outcome = Some(IbusExpansionOutcome::UnknownPartialEmission);
        }
        self.output_retired = true;
        self.enabled = false;
        self.reset();
    }

    /// Complete a synchronous result after the host has emitted every output
    /// action successfully. This acknowledges signal emission only; IBus
    /// provides no receipt that the client document applied those signals.
    pub fn output_succeeded(&mut self) {
        if let Some(result) = self.pending_output.take() {
            self.note(|telemetry, field| telemetry.expansion(field));
            self.engine.commit_applied_expansion(&result);
            self.last_expansion_outcome = Some(IbusExpansionOutcome::EmittedUnverified);
        }
    }

    pub fn last_expansion_outcome(&self) -> Option<IbusExpansionOutcome> {
        self.last_expansion_outcome
    }

    /// Process an IBus key press. `keyval` is an XKB keysym, as specified by
    /// `org.freedesktop.IBus.Engine.ProcessKeyEvent`.
    pub fn process_key_event(&mut self, keyval: u32, keycode: u32, state: u32) -> IbusKeyResult {
        if state & IBUS_RELEASE_MASK == 0 {
            let captured = self.enabled
                && self.capability_block.is_none()
                && self.advertises_surrounding_text()
                && !self.engine.is_sensitive_focus()
                && !self.engine.is_user_paused();
            self.note(|telemetry, field| telemetry.key(field, captured));
        }
        let result = self.process_key_event_inner(keyval, keycode, state);
        if state & IBUS_RELEASE_MASK == 0 {
            if result.handled {
                self.record_emitted(&result.actions);
            } else {
                // The client applies this key itself; the model cannot know
                // its effect until the client reports surrounding text again.
                self.surrounding = None;
            }
        }
        result
    }

    fn process_key_event_inner(&mut self, keyval: u32, _keycode: u32, state: u32) -> IbusKeyResult {
        // IBus delivers both press and release events through this method.
        // Releases carry IBUS_RELEASE_MASK and must not be interpreted as a
        // second printable character or delimiter.
        if state & IBUS_RELEASE_MASK != 0 {
            return IbusKeyResult::default();
        }

        if !self.enabled || self.capability_block.is_some() {
            return IbusKeyResult::default();
        }

        // Without the advertised surrounding-text capability, this engine
        // cannot safely replace a trigger in the target. Pass every event
        // through untouched instead of consuming characters and only
        // discovering the missing contract when a trigger matches.
        if !self.advertises_surrounding_text() {
            self.engine.process(InputEvent::Reset);
            return IbusKeyResult::default();
        }

        // Pre-flight policy check: prevents side effects (e.g., command execution)
        // before policy approval. Post-execution checks happen after engine.process().
        if policy::blocks(
            &self.policy,
            wayexpand_core::pre_flight_check(&self.policy),
            "pre-flight check",
        ) {
            return IbusKeyResult::default();
        }
        // When the core has disabled capture (password fields or an explicit
        // pause), do not consume or commit anything on the client's behalf.
        // The toolkit must receive the original key unchanged.
        if self.engine.is_sensitive_focus() || self.engine.is_user_paused() {
            return IbusKeyResult::default();
        }

        // IBus modifier flags use the same low bits as X11. Do not consume
        // shortcuts or navigation keys: clearing the matcher state is safer
        // than allowing a trigger to span an unrelated command. Mod4 is the
        // normal X11 representation of Super, while IBUS_SUPER_MASK is a
        // separate virtual modifier used by some clients. Mod5 is commonly
        // AltGr, so it is intentionally not treated as a shortcut mask: IBus
        // has already resolved the keyval and the resulting text must remain
        // usable on AltGr layouts.
        const CONTROL_MASK: u32 = 1 << 2;
        const ALT_MASK: u32 = 1 << 3;
        const MOD3_MASK: u32 = 1 << 5;
        const MOD4_MASK: u32 = 1 << 6;
        const IBUS_SUPER_MASK: u32 = 1 << 26;
        if state & (CONTROL_MASK | ALT_MASK | MOD3_MASK | MOD4_MASK | IBUS_SUPER_MASK) != 0 {
            self.engine.process(InputEvent::Reset);
            return IbusKeyResult::default();
        }

        if keyval == xkeysym::key::BackSpace {
            self.engine.process(InputEvent::Backspace);
            return IbusKeyResult::default();
        }

        // Shift and alternate-level selectors can occur in the middle of a
        // printable trigger on layouts such as French AZERTY (`2` is
        // Shift+2). They do not insert text themselves, so preserve the
        // partial trigger while the following key carries the resolved
        // keysym. Shortcut modifiers remain handled by the state-mask check
        // above and still reset matching.
        if is_character_level_modifier(keyval) {
            return IbusKeyResult::default();
        }

        let Some(character) = keysym_to_char(keyval) else {
            self.engine.process(InputEvent::Reset);
            return IbusKeyResult::default();
        };

        let delimiter =
            character.is_whitespace() || !character.is_alphanumeric() && character != '_';
        let event = if delimiter {
            Some(InputEvent::Delimiter(character))
        } else {
            None
        };
        // Deferred execution: policy check BEFORE command execution
        // This prevents side effects from occurring before approval.
        let pending = match event {
            Some(event) => self.engine.process_deferred(event),
            None => self.engine.process_deferred_char(character),
        };
        let mut actions = Vec::new();
        let mut policy_blocked = false;
        for pending_result in pending {
            let has_command = pending_result.command.is_some();

            if let Some(command) = &pending_result.command {
                if command.action.is_none()
                    && policy::blocks(
                        &self.policy,
                        self.policy.command_path_violation(&command.program),
                        "command path",
                    )
                {
                    self.engine
                        .restore_deferred_match(&pending_result.matched_text);
                    policy_blocked = true;
                    continue;
                }
            }

            // Check policy BEFORE executing commands
            let violation = self.policy.expansion_policy_violation(
                pending_result.template_text.len(),
                has_command,
                IBUS_BACKEND_NAME,
            );
            if policy::blocks(&self.policy, violation, "expansion (pre-execution)") {
                self.note(|telemetry, field| telemetry.refusal(field, IbusRefusal::Policy));
                self.engine
                    .restore_deferred_match(&pending_result.matched_text);
                policy_blocked = true;
                continue;
            }

            // Policy-approved commands are queued so ProcessKeyEvent never
            // waits for command timeout. Static matches and cache hits commit
            // synchronously because they require no child process.
            let enforcement_policy = self.policy.effective_enforcement_policy();
            let dispatch = match self.engine.dispatch_pending_with_policy(
                pending_result,
                enforcement_policy.max_replacement_size,
            ) {
                Ok(dispatch) => dispatch,
                Err(e) => {
                    warn!("IBus expansion could not be queued or completed: {}", e);
                    continue;
                }
            };

            match dispatch {
                PendingExpansionDispatch::Ready(result) => {
                    // The current key has not reached the client. Refuse the
                    // replacement unless the client's own text confirms the
                    // rest of the trigger sits right before the cursor.
                    let (delivered, replacement) = replacement_actions(&result, false);
                    if !self.surrounding_confirms(delivered.as_str()) {
                        self.note_surrounding_refusal();
                        warn!(
                            "IBus surrounding text does not confirm the trigger; replacement refused"
                        );
                        self.engine.restore_deferred_match(&result.matched_text);
                        policy_blocked = true;
                        continue;
                    }
                    if injector_capabilities()
                        .replacement_admission(self.engine.compatibility_mode())
                        == ReplacementAdmission::Refused
                    {
                        self.note(|telemetry, field| telemetry.refusal(field, IbusRefusal::Policy));
                        self.engine.restore_deferred_match(&result.matched_text);
                        policy_blocked = true;
                        continue;
                    }
                    actions.extend(replacement);
                    self.pending_output = Some(result);
                }
                PendingExpansionDispatch::Queued => {
                    // The current key has not reached the client yet. Commit
                    // it now so the eventual surrounding-text deletion covers
                    // the complete trigger and preserves key order.
                    actions.push(IbusAction::CommitText(character.to_string().into()));
                }
            }
        }

        if policy_blocked {
            // The trigger characters have already been committed as ordinary
            // IBus text. Do not leave a partial matcher after a blocked
            // replacement; the current key is handled by the normal fallback
            // below and the next trigger starts from a clean boundary.
            self.engine.process(InputEvent::Reset);
            actions.clear();
        }

        if actions.is_empty() {
            // IBus engines must commit ordinary text themselves once they
            // claim the key; this avoids duplicate delivery to the client.
            if !delimiter {
                actions.push(IbusAction::CommitText(character.to_string().into()));
                return IbusKeyResult {
                    handled: true,
                    actions,
                };
            }
            return IbusKeyResult {
                handled: false,
                actions,
            };
        }
        IbusKeyResult {
            handled: true,
            actions,
        }
    }
}

fn is_character_level_modifier(keysym: u32) -> bool {
    matches!(
        keysym,
        xkeysym::key::Shift_L
            | xkeysym::key::Shift_R
            | xkeysym::key::Shift_Lock
            | xkeysym::key::ISO_Level2_Latch
            | xkeysym::key::ISO_Level3_Shift
            | xkeysym::key::ISO_Level3_Latch
            | xkeysym::key::ISO_Level3_Lock
            | xkeysym::key::ISO_Level5_Shift
            | xkeysym::key::ISO_Level5_Latch
            | xkeysym::key::ISO_Level5_Lock
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::{Config, ExpansionEngine, OrganizationPolicy};

    /// Put an adapter in the state ibus-daemon leaves it in for an ordinary
    /// text field: focused, surrounding text supported, free-form content.
    fn focused(mut adapter: IbusEngineAdapter) -> IbusEngineAdapter {
        adapter
            .engine
            .set_compatibility_mode(wayexpand_core::CompatibilityMode::TrackedFocus);
        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        adapter
    }

    #[test]
    fn form_target_is_bound_to_the_focused_ibus_input_context() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ":incident"
replacement = "Incident {{field:hostname}}"
"#,
        )
        .unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());

        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        let without_context = adapter.engine().explain(":incident", IBUS_BACKEND_NAME);
        assert_eq!(
            without_context.suppressed_by().map(|check| check.name),
            Some("target identity"),
            "{}",
            without_context.render_text()
        );

        let context_path = "/org/freedesktop/IBus/Engine/WayExpand_42";
        adapter.focus_in_for_context(Some(context_path));
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        let focused = adapter.engine().explain(":incident", IBUS_BACKEND_NAME);
        assert!(focused.would_expand(), "{}", focused.render_text());
        assert!(focused.checks.iter().any(|check| {
            check.name == "replacement assurance"
                && check.status == wayexpand_core::CheckStatus::Pass
        }));
        assert!(focused.checks.iter().any(|check| {
            check.name == "target identity"
                && check.status == wayexpand_core::CheckStatus::Pass
                && check.detail.contains("exact original input target")
        }));

        adapter.focus_out();
        let blurred = adapter.engine().explain(":incident", IBUS_BACKEND_NAME);
        assert!(!blurred.would_expand(), "{}", blurred.render_text());
        assert!(blurred.checks.iter().any(|check| {
            check.name == "target identity" && check.status == wayexpand_core::CheckStatus::Fail
        }));
    }

    #[test]
    fn app_filter_requires_focus_bound_to_the_same_authenticated_gnome_window() {
        let config = Config::parse(
            r#"[[expansion]]
trigger = ";;ticket"
replacement = "Investigating this now."
app_filter = ["app_id_exact:org.gnome.TextEditor"]
"#,
        )
        .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        adapter.focus_in_for_context(Some("/org/freedesktop/IBus/Engine/WayExpand/7"));

        let editor = WindowContext {
            app_id: Some("org.gnome.TextEditor".into()),
            title: None,
            instance_id: Some("mutter:42".into()),
        };
        adapter.bind_gnome_focus(Some(&editor));
        let allowed = adapter.engine().explain(";;ticket", IBUS_BACKEND_NAME);
        assert!(allowed.would_expand(), "{}", allowed.render_text());

        let elsewhere = WindowContext {
            app_id: Some("org.gnome.Terminal".into()),
            title: None,
            instance_id: Some("mutter:43".into()),
        };
        adapter.update_gnome_focus(Some(&elsewhere));
        let denied = adapter.engine().explain(";;ticket", IBUS_BACKEND_NAME);
        assert!(!denied.would_expand(), "{}", denied.render_text());

        adapter.update_gnome_focus(None);
        let unavailable = adapter.engine().explain(";;ticket", IBUS_BACKEND_NAME);
        assert!(!unavailable.would_expand(), "{}", unavailable.render_text());

        // Even the right app ID is insufficient if the Shell window identity
        // changed after IBus FocusIn; a fresh focus lifecycle is required.
        adapter.update_gnome_focus(Some(&editor));
        let stale_binding = adapter.engine().explain(";;ticket", IBUS_BACKEND_NAME);
        assert!(
            !stale_binding.would_expand(),
            "{}",
            stale_binding.render_text()
        );
    }

    #[cfg(unix)]
    #[test]
    fn form_completes_after_the_original_ibus_context_regains_focus() {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreFormHelperEnv(Option<std::ffi::OsString>);
        impl Drop for RestoreFormHelperEnv {
            fn drop(&mut self) {
                if let Some(value) = self.0.take() {
                    std::env::set_var("WAYEXPAND_FORM_HELPER", value);
                } else {
                    std::env::remove_var("WAYEXPAND_FORM_HELPER");
                }
            }
        }

        let _env_lock = FORM_HELPER_ENV_LOCK.lock().unwrap();
        let helper = std::env::temp_dir().join(format!(
            "wayexpand-ibus-form-helper-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &helper,
            "#!/bin/sh\nprintf '%s' '{\"field:hostname\":\"node-a\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _restore_env = RestoreFormHelperEnv(std::env::var_os("WAYEXPAND_FORM_HELPER"));
        std::env::set_var("WAYEXPAND_FORM_HELPER", &helper);

        let config = Config::parse(
            r#"[[expansion]]
trigger = ":incident"
replacement = "Incident {{field:hostname}}"
"#,
        )
        .unwrap();
        let context_path = "/org/freedesktop/IBus/Engine/WayExpand_form_test";
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        adapter
            .engine_mut()
            .set_compatibility_mode(wayexpand_core::CompatibilityMode::TrackedFocus);
        assert_eq!(
            adapter.engine().compatibility_mode(),
            wayexpand_core::CompatibilityMode::TrackedFocus
        );
        adapter.focus_in_for_context(Some(context_path));
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        let mut client = Client::default();
        client.type_text(&mut adapter, ":incident");

        // Opening the helper temporarily moves focus away from the original
        // IBus context. The same context must be able to resume and receive
        // the result, while surrounding-text verification still guards the
        // actual replacement.
        adapter.focus_out();
        adapter.focus_in_for_context(Some(context_path));
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut replacement = Vec::new();
        while Instant::now() < deadline {
            adapter.set_surrounding_text(
                &client.text(),
                client.cursor as u32,
                client.cursor as u32,
            );
            replacement = adapter.drain_completed_commands_for_test();
            if !replacement.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(adapter);
        let _ = std::fs::remove_file(&helper);

        assert!(
            replacement.iter().any(|action| matches!(
                action,
                IbusAction::CommitText(text) if text == "Incident node-a"
            )),
            "form completion was not delivered: {replacement:?}"
        );
        client.apply(&replacement);
        assert_eq!(client.text(), "Incident node-a");
    }

    /// A minimal IBus client. Like ibus-gtk it reports surrounding text before
    /// every key press, inserts keys the engine does not handle, and applies
    /// the engine's delete/commit actions to its own document.
    #[derive(Default)]
    struct Client {
        text: Vec<char>,
        cursor: usize,
    }

    impl Client {
        fn key(
            &mut self,
            adapter: &mut IbusEngineAdapter,
            keyval: u32,
            state: u32,
        ) -> IbusKeyResult {
            let text: String = self.text.iter().collect();
            adapter.set_surrounding_text(&text, self.cursor as u32, self.cursor as u32);
            let result = adapter.process_key_event(keyval, 0, state);
            if state & IBUS_RELEASE_MASK == 0 && !result.handled {
                if keyval == xkeysym::key::BackSpace {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.text.remove(self.cursor);
                    }
                } else if let Some(character) = keysym_to_char(keyval) {
                    self.text.insert(self.cursor, character);
                    self.cursor += 1;
                }
            }
            self.apply(&result.actions);
            if result.handled {
                adapter.output_succeeded();
            }
            result
        }

        fn type_text(&mut self, adapter: &mut IbusEngineAdapter, text: &str) -> Vec<IbusAction> {
            text.chars()
                .flat_map(|character| self.key(adapter, character as u32, 0).actions)
                .collect()
        }

        fn apply(&mut self, actions: &[IbusAction]) {
            for action in actions {
                match action {
                    IbusAction::DeleteSurroundingText { nchars } => {
                        let start = self.cursor - *nchars as usize;
                        self.text.drain(start..self.cursor);
                        self.cursor = start;
                    }
                    IbusAction::CommitText(text) => {
                        for character in text.chars() {
                            self.text.insert(self.cursor, character);
                            self.cursor += 1;
                        }
                    }
                }
            }
        }

        fn text(&self) -> String {
            self.text.iter().collect()
        }
    }

    fn adapter() -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[settings]
compatibility_mode = "tracked-focus"

[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ))
    }

    #[test]
    fn default_policy_refuses_non_atomic_replacement() {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\n")
                .unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        adapter.set_content_type(0, 0);
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), ":sig");
        assert_eq!(adapter.last_expansion_outcome(), None);
    }

    #[test]
    fn failed_signal_sequence_does_not_commit_expansion_accounting() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":si");
        let text = client.text();
        adapter.set_surrounding_text(&text, 3, 3);
        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert!(matches!(
            result.actions.first(),
            Some(IbusAction::DeleteSurroundingText { .. })
        ));
        // Model the D-Bus failure after DeleteSurroundingText but before
        // CommitText: the host cannot claim the expansion was applied.
        let failed_after_delete = result.actions.len() > 1;
        assert!(failed_after_delete);
        adapter.output_failed();
        assert_eq!(
            adapter.last_expansion_outcome(),
            Some(IbusExpansionOutcome::UnknownPartialEmission)
        );
        let undo = wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap();
        assert!(adapter.engine().prepare_undo(&undo).is_none());
    }

    #[test]
    fn component_file_detection_is_independent_of_ibus_registry_refresh() {
        let root =
            std::env::temp_dir().join(format!("wayexpand-ibus-component-{}", std::process::id()));
        let component_dir = root.join("ibus/component");
        std::fs::create_dir_all(&component_dir).unwrap();
        assert!(!component_file_present_in(std::slice::from_ref(
            &component_dir
        )));
        std::fs::write(component_dir.join("wayexpand-ibus.xml"), "<component/>").unwrap();
        assert!(
            component_file_present_in(std::slice::from_ref(&component_dir)),
            "packaged manifest name must be recognized"
        );
        std::fs::remove_file(component_dir.join("wayexpand-ibus.xml")).unwrap();
        std::fs::write(component_dir.join("wayexpand.xml"), "<component/>").unwrap();
        assert!(component_file_present_in(std::slice::from_ref(
            &component_dir
        )));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn boundary_adapter() -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[settings]
undo_chord = "Ctrl+Z"

[[expansion]]
trigger = ":sig"
replacement = "signature"
match_mode = "word-boundary"
"#,
        )
        .unwrap();
        focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ))
    }

    fn policy_adapter(policy: OrganizationPolicy) -> IbusEngineAdapter {
        let config: Config = toml::from_str(
            r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
"#,
        )
        .unwrap();
        focused(
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap(),
        )
    }

    #[test]
    fn administrator_absolute_command_policy_rejects_relative_programs() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["ok"]
            "#,
        )
        .unwrap();
        let engine = ExpansionEngine::new(config).unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };

        assert!(matches!(
            IbusEngineAdapter::with_policy(engine, policy),
            Err(wayexpand_core::ConfigError::InvalidCommand { .. })
        ));
    }

    #[test]
    fn ibus_disables_commands_even_in_audit_mode() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            [expansion.command]
            program = "printf"
            args = ["audit-ok"]
            "#,
        )
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();
        assert!(adapter.engine().direct_commands_disabled());

        let mut key_actions = Vec::new();
        for character in ":cmd".chars() {
            key_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert!(key_actions.iter().all(|action| matches!(
            action,
            IbusAction::CommitText(text) if text.chars().count() == 1
        )));
        assert!(adapter.drain_completed_commands_for_test().is_empty());
    }

    #[test]
    fn ibus_keeps_managed_actions_enabled_behind_the_broker_boundary() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":action"
            replacement = ""
            [expansion.command]
            action = "cluster-status"
            timeout_ms = 3000
            "#,
        )
        .unwrap();
        let adapter = IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap());
        assert!(adapter.engine().direct_commands_disabled());
        assert!(!adapter.engine().commands_disabled());
    }

    #[cfg(unix)]
    #[test]
    fn command_backed_expansions_are_not_started_by_ibus() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":slow"
            replacement = ""
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "sleep 0.4; printf done"]
            timeout_ms = 1000
            "#,
        )
        .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        let started = Instant::now();
        let mut key_actions = Vec::new();
        for character in ":slow".chars() {
            key_actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "IBus key processing waited for the child process"
        );
        assert!(key_actions.contains(&IbusAction::CommitText("w".into())));

        assert!(adapter.drain_completed_commands_for_test().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn deferred_command_output_over_organization_limit_is_not_committed() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-ibus-policy-output-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let command = format!(
            "printf completed > '{}'; yes x | head -c 257",
            marker.display()
        );
        let config = Config::parse(&format!(
            r#"
            [[expansion]]
            trigger = ":large"
            replacement = ""
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "{command}"]
            timeout_ms = 1000
            "#
        ))
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: true,
            max_replacement_size: 256,
            ..OrganizationPolicy::default()
        };
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();
        assert!(adapter.engine_mut().enable_async_commands());

        let mut actions = Vec::new();
        for character in ":large".chars() {
            actions.extend(adapter.process_key_event(character as u32, 0, 0).actions);
        }

        actions.extend(adapter.drain_completed_commands_for_test());
        thread::sleep(Duration::from_millis(20));
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, IbusAction::CommitText(text) if text.len() > 1)),
            "command output must not be committed by IBus"
        );
        assert!(
            !marker.exists(),
            "IBus must not spawn command-backed expansions"
        );
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn async_workers_are_lazy_and_follow_the_active_config_generation() {
        let text_config =
            Config::parse("[[expansion]]\ntrigger = ':hello'\nreplacement = 'hello'\n").unwrap();
        let mut adapter = IbusEngineAdapter::new(ExpansionEngine::new(text_config).unwrap());
        assert!(!adapter.engine().async_commands_enabled());
        assert_eq!(adapter.engine().command_metrics().command_worker_threads, 0);
        assert_eq!(adapter.engine().command_metrics().hotkey_worker_threads, 0);

        let command_config = Config::parse(
            "[[expansion]]\ntrigger = ':cmd'\nreplacement = ''\n\
             [expansion.command]\nprogram = '/usr/bin/true'\nargs = []\n",
        )
        .unwrap();
        adapter.replace_config(command_config).unwrap();
        assert!(adapter.engine().async_commands_enabled());
        let metrics = adapter.engine().command_metrics();
        assert_eq!(metrics.command_worker_threads, 4);
        assert_eq!(metrics.hotkey_worker_threads, 1);

        adapter
            .replace_config(
                Config::parse("[[expansion]]\ntrigger = ':hello'\nreplacement = 'hello'\n")
                    .unwrap(),
            )
            .unwrap();
        assert!(!adapter.engine().async_commands_enabled());
        let metrics = adapter.engine().command_metrics();
        assert_eq!(metrics.command_worker_threads, 0);
        assert_eq!(metrics.hotkey_worker_threads, 0);
    }

    #[cfg(unix)]
    #[test]
    fn audit_mode_logs_but_commits_async_output_over_organization_limit() {
        let config = Config::parse(
            r#"
            [[expansion]]
            trigger = ":large"
            replacement = ""
            match_mode = "word-boundary"
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "yes x | head -c 257"]
            timeout_ms = 1000
            "#,
        )
        .unwrap();
        let policy = OrganizationPolicy {
            safe_mode: false,
            max_replacement_size: 256,
            ..OrganizationPolicy::default()
        };
        let mut adapter =
            IbusEngineAdapter::with_policy(ExpansionEngine::new(config).unwrap(), policy).unwrap();

        for character in ":large".chars() {
            adapter.process_key_event(character as u32, 0, 0);
        }
        let actions = adapter.process_key_event(' ' as u32, 0, 0).actions;
        assert!(actions.iter().all(|action| !matches!(
            action,
            IbusAction::CommitText(text) if text.len() > 1
        )));
        assert!(adapter.drain_completed_commands_for_test().is_empty());
    }

    #[test]
    fn ordinary_text_is_committed_immediately() {
        let mut adapter = adapter();
        let result = adapter.process_key_event('a' as u32, 0, 0);
        assert!(result.handled);
        assert_eq!(result.actions, vec![IbusAction::CommitText("a".into())]);
    }

    #[test]
    fn trigger_is_deleted_and_replacement_committed() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, "x");
        let actions = client.type_text(&mut adapter, ":sig");
        // The final `g` is consumed and never reaches the client, so only
        // `:si` is deleted. Deleting four would also remove the user's `x`.
        assert_eq!(
            &actions[actions.len() - 2..],
            &[
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("signature".into())
            ]
        );
        assert_eq!(client.text(), "xsignature");
    }

    #[test]
    fn unicode_keysym_forms_are_committed_without_loss() {
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(
                Config::parse(
                    r#"[[expansion]]
trigger = ":東京😀"
replacement = "世界 🌍"
"#,
                )
                .unwrap(),
            )
            .unwrap(),
        ));

        // IBus uses the X11 Unicode keysym form for code points outside the
        // legacy Latin-1 range: 0x01000000 + the scalar value.
        let unicode_keysym = |character: char| 0x0100_0000 + character as u32;
        let mut client = Client::default();
        let mut actions = Vec::new();
        for character in ":東京😀".chars() {
            let keyval = if character.is_ascii() {
                character as u32
            } else {
                unicode_keysym(character)
            };
            actions.extend(client.key(&mut adapter, keyval, 0).actions);
        }

        assert_eq!(
            actions,
            vec![
                IbusAction::CommitText("東".into()),
                IbusAction::CommitText("京".into()),
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("世界 🌍".into()),
            ]
        );
        assert_eq!(client.text(), "世界 🌍");
    }

    #[test]
    fn invalid_unicode_keysym_is_not_committed() {
        let mut adapter = adapter();
        let result = adapter.process_key_event(0x0100_0000 + 0x11_0000, 0, 0);
        assert_eq!(result, IbusKeyResult::default());
    }

    #[test]
    fn synchronous_ibus_expansion_commits_undo_after_actions_are_created() {
        let config: Config = toml::from_str(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[expansion]]
            trigger = ":sig"
            replacement = "signature"
            "#,
        )
        .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        Client::default().type_text(&mut adapter, ":sig");

        let undo = adapter
            .engine()
            .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
            .expect("a successfully emitted IBus expansion should be undoable");
        assert_eq!(undo.matched_text, "signature");
        assert_eq!(undo.insert, ":sig");
    }

    #[test]
    fn delimiter_is_reinserted_with_the_replacement() {
        let mut adapter = boundary_adapter();
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig ");
        assert_eq!(
            &actions[actions.len() - 2..],
            &[
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("signature ".into())
            ]
        );
        assert_eq!(client.text(), "signature ");
        let undo = adapter
            .engine()
            .prepare_undo(&wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap())
            .expect("a boundary expansion should be undoable");
        assert_eq!(undo.matched_text, "signature ");
        assert_eq!(undo.insert, ":sig ");
    }

    #[cfg(unix)]
    #[test]
    fn command_backed_word_boundary_is_forwarded_without_execution() {
        let config: Config = toml::from_str(
            r#"[settings]
undo_chord = "Ctrl+Z"

[[expansion]]
trigger = ":sig"
replacement = ""
match_mode = "word-boundary"
[expansion.command]
program = "/bin/sh"
args = ["-c", "printf signature"]
timeout_ms = 1000
"#,
        )
        .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));

        let typed_actions = Client::default().type_text(&mut adapter, ":sig ");
        assert!(!typed_actions
            .iter()
            .any(|action| matches!(action, IbusAction::DeleteSurroundingText { .. })));
        assert!(!typed_actions.iter().any(|action| matches!(
            action,
            IbusAction::CommitText(text) if text.contains("signature")
        )));
        assert!(adapter.drain_completed_commands_for_test().is_empty());
    }

    #[test]
    fn printable_key_release_is_not_committed() {
        let mut adapter = adapter();

        assert_eq!(
            adapter.process_key_event('a' as u32, 0, IBUS_RELEASE_MASK),
            IbusKeyResult::default()
        );
    }

    #[test]
    fn trigger_key_release_does_not_advance_matcher() {
        let mut adapter = adapter();
        let mut client = Client::default();

        client.type_text(&mut adapter, ":si");
        assert_eq!(
            client.key(&mut adapter, 'i' as u32, IBUS_RELEASE_MASK),
            IbusKeyResult::default()
        );

        let result = client.key(&mut adapter, 'g' as u32, 0);
        assert_eq!(
            result.actions,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 3 },
                IbusAction::CommitText("signature".into())
            ]
        );
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn modifier_key_release_does_not_reset_valid_buffer() {
        const CONTROL_MASK: u32 = 1 << 2;
        let mut adapter = adapter();
        let mut client = Client::default();

        client.type_text(&mut adapter, ":si");
        assert_eq!(
            client.key(
                &mut adapter,
                xkeysym::key::Control_L,
                CONTROL_MASK | IBUS_RELEASE_MASK
            ),
            IbusKeyResult::default()
        );

        let result = client.key(&mut adapter, 'g' as u32, 0);
        assert!(result
            .actions
            .contains(&IbusAction::CommitText("signature".into())));
    }

    #[test]
    fn super_modifiers_do_not_consume_printable_shortcuts() {
        const MOD4_MASK: u32 = 1 << 6;
        const IBUS_SUPER_MASK: u32 = 1 << 26;

        for state in [MOD4_MASK, IBUS_SUPER_MASK] {
            let mut adapter = adapter();
            let result = adapter.process_key_event('a' as u32, 0, state);
            assert_eq!(result, IbusKeyResult::default());
        }
    }

    #[test]
    fn modifier_levels_do_not_consume_boundary_shortcuts() {
        const MOD3_MASK: u32 = 1 << 5;
        const MOD4_MASK: u32 = 1 << 6;
        const MOD5_MASK: u32 = 1 << 7;

        for state in [MOD3_MASK, MOD4_MASK] {
            let mut adapter = boundary_adapter();
            for character in ":sig".chars() {
                adapter.process_key_event(character as u32, 0, 0);
            }
            assert_eq!(
                adapter.process_key_event(' ' as u32, 0, state),
                IbusKeyResult::default()
            );
            assert_eq!(
                adapter.process_key_event('g' as u32, 0, 0).actions,
                vec![IbusAction::CommitText("g".into())]
            );
        }

        let mut altgr_adapter = boundary_adapter();
        let mut client = Client::default();
        client.type_text(&mut altgr_adapter, ":sig");
        assert_eq!(
            client
                .key(&mut altgr_adapter, ' ' as u32, MOD5_MASK)
                .actions,
            vec![
                IbusAction::DeleteSurroundingText { nchars: 4 },
                IbusAction::CommitText("signature ".into())
            ]
        );
    }

    #[test]
    fn engine_instances_do_not_share_matcher_state() {
        let mut first = adapter();
        let mut second = adapter();
        let mut first_client = Client::default();

        first_client.type_text(&mut first, ":si");

        // A second input context must not inherit the first context's partial
        // trigger. Its ordinary key is committed unchanged.
        let isolated = Client::default().key(&mut second, 'g' as u32, 0);
        assert_eq!(isolated.actions, vec![IbusAction::CommitText("g".into())]);

        let completed = first_client.key(&mut first, 'g' as u32, 0);
        assert!(completed
            .actions
            .contains(&IbusAction::CommitText("signature".into())));
    }

    #[test]
    fn safe_organization_policy_blocks_disallowed_ibus_expansion() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig");
        assert_eq!(actions.last(), Some(&IbusAction::CommitText("g".into())));
        assert_eq!(client.text(), ":sig");
    }

    #[test]
    fn audit_organization_policy_allows_but_reports_disallowed_ibus_expansion() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: false,
            allowed_backends: vec!["libei".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn input_method_v2_allowance_does_not_permit_ibus() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["input-method-v2".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), ":sig");

        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            allowed_backends: vec!["input-method-v2".into(), "ibus".into()],
            ..Default::default()
        });
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn safe_mode_atomic_replace_requirement_disables_ibus() {
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            require_atomic_replace: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_some());
        let result = adapter.process_key_event('a' as u32, 0, 0);
        assert_eq!(result, IbusKeyResult::default());

        // Audit mode reports the gap but keeps the route usable.
        let mut adapter = policy_adapter(OrganizationPolicy {
            safe_mode: false,
            require_atomic_replace: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_none());
        let mut client = Client::default();
        client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
    }

    #[test]
    fn sensitive_focus_requirement_is_met_by_content_types() {
        let adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            require_sensitive_focus: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_none());
    }

    #[test]
    fn exclusive_capture_requirement_accepts_ibus_capture() {
        let adapter = policy_adapter(OrganizationPolicy {
            safe_mode: true,
            require_exclusive_capture: true,
            ..Default::default()
        });
        assert!(adapter.capability_block().is_none());
    }

    #[test]
    fn ibus_route_does_not_claim_atomic_replacement() {
        assert!(!injector_capabilities().atomic_replace);
        assert_eq!(
            injector_capabilities().replacement_guarantee,
            wayexpand_core::ReplacementGuarantee::VerifiedSurroundingText
        );
    }

    fn unfocused_adapter() -> IbusEngineAdapter {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\n")
                .unwrap();
        IbusEngineAdapter::new(ExpansionEngine::new(config).unwrap())
    }

    #[test]
    fn capture_is_off_until_a_content_type_arrives() {
        let mut adapter = unfocused_adapter();
        assert!(adapter.engine().is_sensitive_focus());
        adapter.focus_in();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);
        // A key that arrives before SetContentType passes through untouched.
        assert_eq!(
            adapter.process_key_event('a' as u32, 0, 0),
            IbusKeyResult::default()
        );
        adapter.set_content_type(0, 0);
        assert!(!adapter.engine().is_sensitive_focus());
        assert!(adapter.process_key_event('a' as u32, 0, 0).handled);
    }

    #[test]
    fn sensitive_content_types_and_hints_disable_capture() {
        const PASSWORD: u32 = 8;
        const PIN: u32 = 9;
        const TERMINAL: u32 = 10;
        const DATETIME: u32 = 13;
        const UNKNOWN: u32 = 14;
        const PRIVATE: u32 = 1 << 11;
        const HIDDEN_TEXT: u32 = 1 << 12;
        for (purpose, hints, sensitive) in [
            (0, 0, false),
            (TERMINAL, 0, false),
            (DATETIME, 0, false),
            (PASSWORD, 0, true),
            (PIN, 0, true),
            (UNKNOWN, 0, true),
            (u32::MAX, 0, true),
            (0, PRIVATE, true),
            (0, HIDDEN_TEXT, true),
            (0, 1 << 0, false),
        ] {
            assert_eq!(
                content_type_is_sensitive(purpose, hints),
                sensitive,
                "purpose {purpose} hints {hints:#x}"
            );
            let mut adapter = adapter();
            adapter.set_content_type(purpose, hints);
            assert_eq!(adapter.engine().is_sensitive_focus(), sensitive);
            if sensitive {
                assert_eq!(
                    adapter.process_key_event('a' as u32, 0, 0),
                    IbusKeyResult::default()
                );
            }
        }
    }

    #[test]
    fn refocus_without_a_new_content_type_keeps_the_last_one() {
        // ibus-daemon does not set ContentType again when the newly focused
        // field has the same type as the last one it set on this engine.
        let mut adapter = adapter();
        assert!(!adapter.engine().is_sensitive_focus());
        adapter.focus_out();
        assert!(adapter.engine().is_sensitive_focus());
        adapter.focus_in();
        assert!(!adapter.engine().is_sensitive_focus());
        assert!(adapter.process_key_event('a' as u32, 0, 0).handled);
    }

    #[test]
    fn refocus_keeps_a_sensitive_content_type_sensitive() {
        let mut adapter = adapter();
        adapter.set_content_type(8, 0);
        adapter.focus_out();
        adapter.focus_in();
        assert!(adapter.engine().is_sensitive_focus());
        assert_eq!(
            adapter.process_key_event('a' as u32, 0, 0),
            IbusKeyResult::default()
        );
    }

    #[test]
    fn refocus_before_any_content_type_stays_sensitive() {
        let mut adapter = unfocused_adapter();
        adapter.focus_in();
        adapter.focus_out();
        adapter.focus_in();
        assert!(adapter.engine().is_sensitive_focus());
    }

    #[test]
    fn reset_keeps_the_field_content_type() {
        let mut adapter = adapter();
        adapter.set_content_type(8, 0);
        adapter.reset();
        assert!(adapter.engine().is_sensitive_focus());
        adapter.set_content_type(0, 0);
        adapter.reset();
        assert!(!adapter.engine().is_sensitive_focus());
    }

    #[test]
    fn moved_cursor_without_reset_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":s");
        // The user clicks elsewhere; this client sends no Reset.
        client.cursor = 0;
        let actions = client.type_text(&mut adapter, "ig");
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, IbusAction::DeleteSurroundingText { .. })),
            "{actions:?}"
        );
        assert_eq!(client.text(), "ig:s");
    }

    #[test]
    fn active_selection_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":si");
        let text = client.text();
        adapter.set_surrounding_text(&text, 3, 1);
        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert_eq!(result.actions, vec![IbusAction::CommitText("g".into())]);
    }

    #[test]
    fn client_without_surrounding_text_gets_no_replacement() {
        let mut adapter = adapter();
        adapter.set_capabilities(0);
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig");
        assert!(actions.is_empty(), "unverified input should pass through");
        assert_eq!(client.text(), ":sig");
    }

    #[test]
    fn ordinary_text_is_committed_when_surrounding_text_is_supported() {
        let mut adapter = adapter();
        adapter.set_capabilities(IBUS_CAP_SURROUNDING_TEXT);

        let result = adapter.process_key_event('x' as u32, 0, 0);

        assert!(result.handled);
        assert_eq!(result.actions, vec![IbusAction::CommitText("x".into())]);
    }

    #[test]
    fn stale_surrounding_text_refuses_replacement() {
        let mut adapter = adapter();
        let mut client = Client::default();
        client.type_text(&mut adapter, ":si");
        // The client reports text that no longer ends with the trigger.
        adapter.set_surrounding_text("hello", 5, 5);
        let result = adapter.process_key_event('g' as u32, 0, 0);
        assert_eq!(result.actions, vec![IbusAction::CommitText("g".into())]);
    }

    #[test]
    fn legacy_and_keypad_keysyms_convert_to_text() {
        for (keysym, expected) in [
            (0x07e1, Some('α')), // Greek_alpha
            (0x06c1, Some('а')), // Cyrillic_a
            (0x05c7, Some('ا')), // Arabic_alef
            (0x0ce0, Some('א')), // hebrew_aleph
            (xkeysym::key::KP_1, Some('1')),
            (xkeysym::key::KP_Space, Some(' ')),
            (xkeysym::key::Return, Some('\n')),
            (xkeysym::key::Tab, Some('\t')),
            (xkeysym::key::dead_acute, None),
            (xkeysym::key::Multi_key, None),
            (xkeysym::key::Escape, None),
            (xkeysym::key::Delete, None),
            (xkeysym::key::BackSpace, None),
        ] {
            assert_eq!(keysym_to_char(keysym), expected, "keysym {keysym:#x}");
        }
    }

    #[test]
    fn greek_trigger_typed_with_legacy_keysyms_expands() {
        let config: Config =
            toml::from_str("[[expansion]]\ntrigger = \";αβ\"\nreplacement = \"alpha beta\"\n")
                .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        let mut client = Client::default();
        client.key(&mut adapter, ';' as u32, 0);
        client.key(&mut adapter, 0x07e1, 0); // Greek_alpha
        client.key(&mut adapter, 0x07e2, 0); // Greek_beta
        assert_eq!(client.text(), "alpha beta");
    }

    #[test]
    fn shifted_character_inside_trigger_does_not_reset_matching() {
        const SHIFT_MASK: u32 = 1;
        let config = Config::parse(
            "[settings]\ncompatibility_mode = 'tracked-focus'\n\
             [[expansion]]\ntrigger = ';e2e'\nreplacement = 'expanded'\n",
        )
        .unwrap();
        let mut adapter = focused(IbusEngineAdapter::new(
            ExpansionEngine::new(config).unwrap(),
        ));
        let mut client = Client::default();

        client.key(&mut adapter, ';' as u32, 0);
        client.key(&mut adapter, 'e' as u32, 0);
        client.key(&mut adapter, xkeysym::key::Shift_L, 0);
        client.key(&mut adapter, '2' as u32, SHIFT_MASK);
        client.key(
            &mut adapter,
            xkeysym::key::Shift_L,
            SHIFT_MASK | IBUS_RELEASE_MASK,
        );
        client.key(&mut adapter, 'e' as u32, 0);

        assert_eq!(client.text(), "expanded");
    }

    #[test]
    fn output_failure_disables_matching_until_a_new_focus_lifecycle() {
        let mut adapter = adapter();
        adapter.output_failed();
        adapter.enable();
        assert!(!adapter.process_key_event(':' as u32, 0, 0).handled);

        // IBus calls FocusIn, Enable, capabilities, and content type for a
        // newly focused context. Matching may resume only after that sequence.
        let mut adapter = focused(adapter);
        let mut client = Client::default();
        let actions = client.type_text(&mut adapter, ":sig");
        assert_eq!(client.text(), "signature");
        assert!(actions.iter().any(|action| matches!(
            action,
            IbusAction::CommitText(text) if text == "signature"
        )));
    }

    #[test]
    fn adapters_share_validated_generation_and_matcher_but_isolate_input_state() {
        let config =
            Config::parse("[[expansion]]\ntrigger = ':hello'\nreplacement = 'hello'\n").unwrap();
        let validated = Arc::new(config.validate_snapshot().unwrap());
        let first = IbusEngineAdapter::with_validated_config(
            Arc::clone(&validated),
            OrganizationPolicy::default(),
        );
        let second = IbusEngineAdapter::with_validated_config(
            Arc::clone(&validated),
            OrganizationPolicy::default(),
        );

        assert!(Arc::ptr_eq(first.engine.validated_config(), &validated));
        assert!(Arc::ptr_eq(second.engine.validated_config(), &validated));
    }
}
