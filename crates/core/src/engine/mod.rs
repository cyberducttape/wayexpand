mod async_runtime;
mod cache;
pub mod command_runtime;
mod commands;
pub mod expansion;
pub mod explain;
mod hotkeys;
mod insertion;
pub mod matching;
mod policy;
mod runtime_state;
mod state;
pub mod transaction;
mod undo;
mod usage;
pub use async_runtime::CompletionNotifier;
pub use commands::{
    BrokerOperation, BrokerProtocolFailure, BrokerUnavailableReason, CommandError,
    CommandErrorKind, CommandMetrics, ExpansionError, HotkeyError, HotkeyResult,
    ProcessWaitFailure, ProcessWaitOperation,
};
pub use explain::{CheckStatus, ExplainCheck, Explanation};
pub use insertion::InsertError;
pub use state::{
    ExpansionResult, InputEvent, MatchPlan, PendingExpansionDispatch, PendingExpansionResult,
    WindowContext,
};
pub use transaction::TransactionOutcome;

use cache::{ClipboardTriggers, CommandCacheEntry};
use matching::GlobPattern;
use state::NormalizedWindowContext;

use async_runtime::{AsyncCommandJob, AsyncCommandRuntime, FormOrigin};
pub use command_runtime::{run_command, run_command_cancellable};
use command_runtime::{CommandMetricsState, QueueSendError};

use crate::{
    AppFilter, CommandConfig, CommandEnvironment, Config, ConfigError, HotkeyConfig,
    InjectorCapabilities, KeyChord, Matcher,
};
use std::{
    collections::VecDeque,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

pub(crate) use crate::limits::{
    MAX_COMMAND_OUTPUT_BYTES, MAX_RESULTS_PER_EVENT, MAX_RESULT_BYTES_PER_EVENT,
};
const MINIMAL_COMMAND_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const ASYNC_COMMAND_QUEUE_CAPACITY: usize = 16;
const ASYNC_COMMAND_WORKER_COUNT: usize = 4;

/// Maximum accepted size for opaque backend-provided toplevel identities.
pub const MAX_WINDOW_INSTANCE_ID_BYTES: usize = 192;

pub struct ExpansionEngine {
    config: Config,
    /// Case-folded app filters are immutable for the lifetime of an engine;
    /// avoid allocating them on every candidate match.
    app_filters: Vec<Vec<AppFilter>>,
    /// Compiled app-id globs corresponding positionally to `app_filters`.
    app_filter_globs: Vec<Vec<Option<GlobPattern>>>,
    matcher: Matcher,
    matcher_indices: Vec<usize>,
    buffer: VecDeque<char>,
    max_buffer_chars: usize,
    /// Set whenever the buffer evicts a character from the front because it
    /// exceeded `max_buffer_chars`, and cleared only by `buffer.clear()`
    /// (a known boundary, e.g. after a match). Lets `match_allowed`
    /// distinguish "truly at the start of input" from "a preceding
    /// character existed but was evicted" -- the two cases an empty
    /// `rev().nth(length)` cannot tell apart on its own.
    buffer_truncated: bool,
    /// User-initiated pause state. Independent from sensitive field detection.
    user_paused: bool,
    /// Compositor/backend signal that focused field is sensitive (password,
    /// OTP, etc.). Independent from user_paused.
    sensitive_focus: bool,
    /// Backend-reported active IME/dead-key/Compose composition state.
    composition_active: bool,
    /// Backend-specific isolation gate for direct executable commands. Named
    /// broker actions use their own IPC boundary and are not blocked by this.
    direct_commands_disabled: bool,
    command_cache: Vec<Option<CommandCacheEntry>>,
    hotkeys: Vec<(KeyChord, usize)>,
    current_window: Option<WindowContext>,
    normalized_window: Option<NormalizedWindowContext>,
    undo_chord: Option<KeyChord>,
    /// The most recent successful expansion, kept only until the very next
    /// event of any other kind (see `process`): `(text to type back,
    /// exact text to erase)`. Not set for a result with a `{{cursor}}`
    /// marker, since undoing after the user has typed more text at that
    /// repositioned cursor has no single well-defined "erase N characters
    /// backward" meaning.
    last_expansion: Option<(String, String)>,
    /// Trigger text removed while a deferred result is being prepared. The
    /// reservation is released only after successful injection; otherwise it
    /// is restored before subsequent input is matched.
    deferred_matches: Vec<String>,
    input_generation: u64,
    shared_input_generation: Arc<AtomicU64>,
    async_commands: Option<AsyncCommandRuntime>,
    /// Called by expansion workers after each completion is queued, so a host
    /// without its own event loop can drain results on wakeup instead of
    /// polling. Shared with running workers so it can be set at any time.
    completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
    expansion_metrics: Arc<CommandMetricsState>,
    hotkey_metrics: Arc<CommandMetricsState>,
    /// Whether a terminating character should be included in the replacement
    /// operation. Exclusive input sources have not delivered the delimiter to
    /// the application yet; non-exclusive sources such as evdev have.
    reinsert_terminators: bool,
    /// Environment and include values for templates, built once per config.
    template_base: crate::TemplateContext,
    /// Supplied by the host when it can read the clipboard.
    clipboard: Option<crate::ClipboardReader>,
    /// Optional host hook that starts a clipboard read early (see
    /// [`ExpansionEngine::set_clipboard_prefetch`]).
    clipboard_prefetch: Option<crate::ClipboardPrefetch>,
    /// Forward trie of the effective triggers of `{{clipboard}}` snippets,
    /// used only to decide when to prefetch. `None` when there are none.
    clipboard_triggers: Option<ClipboardTriggers>,
    /// The prefetch hook already fired for the prefix currently being typed.
    clipboard_prefetch_armed: bool,
    /// Applied expansions not yet collected by the host; see
    /// [`ExpansionEngine::drain_usage_events`].
    usage_events: VecDeque<crate::UsageEvent>,
    /// A snippet form is open; capture is suspended until it completes.
    form_active: bool,
}

impl ExpansionEngine {
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        Self::new_with_snippets(config, None)
    }

    /// Construct an engine using an externally supplied static snippet
    /// library for template validation and rendering. Trigger matching still
    /// compiles only the expansions in `config`.
    pub fn new_with_snippets(
        config: Config,
        snippets: Option<Arc<crate::SnippetLibrary>>,
    ) -> Result<Self, ConfigError> {
        // Build the static snippet library once and share it between
        // validation and rendering; it can hold large replacements.
        let snippets = snippets.unwrap_or_else(|| config.includable_snippets());
        // Triggers match literally (see `Matcher`, a case-sensitive char
        // trie), so a `propagate_case` expansion is matched by inserting
        // its uppercase and capitalized forms as additional trigger
        // strings mapped to the same config entry, rather than by making
        // matching itself case-insensitive (which would affect every
        // expansion, not just ones that opted in). `take_match` later reads
        // back which form was actually typed to decide how to case the
        // replacement. Validation builds these from `effective_triggers()`
        // and rejects collisions across exactly these variants, so its
        // result is reused here instead of being recomputed.
        let enabled = config.validate_with_snippets(Some(Arc::clone(&snippets)))?;
        let matcher_indices = enabled.iter().map(|(index, _)| *index).collect();
        let (clipboard, others): (Vec<_>, Vec<_>) = enabled.iter().partition(|(index, _)| {
            let expansion = &config.expansion[*index];
            expansion.command.is_none()
                && crate::template_variables(&expansion.replacement).contains(&"clipboard")
        });
        let clipboard_triggers = ClipboardTriggers::new(
            clipboard
                .into_iter()
                .map(|(_, trigger)| trigger.clone())
                .collect(),
            others
                .into_iter()
                .map(|(_, trigger)| trigger.clone())
                .collect(),
        );
        let matcher = Matcher::new(enabled.into_iter().map(|(_, trigger)| trigger));
        let app_filters: Vec<Vec<AppFilter>> = config
            .expansion
            .iter()
            .map(|expansion| {
                expansion
                    .app_filter
                    .iter()
                    .filter_map(|filter| AppFilter::parse(filter))
                    .collect()
            })
            .collect();
        let app_filter_globs = app_filters
            .iter()
            .map(|filters| {
                filters
                    .iter()
                    .map(|filter| match filter {
                        AppFilter::AppIdGlob(pattern) => Some(GlobPattern::compile(pattern)),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let template_base = config.template_context_with_snippets(None, snippets);
        let max_buffer_chars = config.settings.max_buffer_chars;
        // `validate()` above already confirmed this parses; a config that
        // fails to load is never used to construct an engine.
        let undo_chord = config
            .settings
            .undo_chord
            .as_ref()
            .and_then(|chord| KeyChord::parse(chord).ok());
        let command_cache = vec![None; config.expansion.len()];
        let hotkeys = config
            .hotkey
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.enabled)
            .filter_map(|(index, binding)| {
                KeyChord::parse(&binding.chord)
                    .ok()
                    .map(|chord| (chord, index))
            })
            .collect();
        Ok(Self {
            config,
            app_filters,
            app_filter_globs,
            matcher,
            matcher_indices,
            buffer: VecDeque::new(),
            max_buffer_chars,
            buffer_truncated: false,
            user_paused: false,
            sensitive_focus: false,
            composition_active: false,
            direct_commands_disabled: false,
            command_cache,
            hotkeys,
            current_window: None,
            normalized_window: None,
            undo_chord,
            last_expansion: None,
            deferred_matches: Vec::new(),
            input_generation: 0,
            shared_input_generation: Arc::new(AtomicU64::new(0)),
            async_commands: None,
            completion_notifier: Arc::new(RwLock::new(None)),
            expansion_metrics: Arc::new(CommandMetricsState::new()),
            hotkey_metrics: Arc::new(CommandMetricsState::new()),
            reinsert_terminators: true,
            template_base,
            clipboard: None,
            clipboard_prefetch: None,
            clipboard_triggers,
            clipboard_prefetch_armed: false,
            usage_events: VecDeque::new(),
            form_active: false,
        })
    }

    /// Replace the library portion of template context without rebuilding the
    /// trigger matcher. Editor previews use this to retain `{{snippet:name}}`
    /// semantics while compiling only the draft trigger.
    pub fn set_template_context(&mut self, context: crate::TemplateContext) {
        self.template_base = context;
    }

    /// Apply administrator-owned policy to an already constructed engine.
    /// This changes policy metadata only; matching state and runtime state are
    /// preserved.
    pub fn apply_administrator_policy(
        &mut self,
        policy: &crate::OrganizationPolicy,
    ) -> Result<(), ConfigError> {
        self.config.apply_administrator_policy(policy)
    }

    /// Configure whether delimiters reported with a match must be reinserted.
    /// Exclusive sources need this enabled; non-exclusive sources (evdev)
    /// should leave the physical delimiter to the focused application.
    pub fn set_reinsert_terminators(&mut self, enabled: bool) {
        self.reinsert_terminators = enabled;
    }

    pub fn reinserts_terminators(&self) -> bool {
        self.reinsert_terminators
    }

    /// Supply the clipboard reader for `{{clipboard}}`. It is used only when
    /// the configuration enables the variable and policy allows it.
    pub fn set_clipboard_reader(&mut self, reader: Option<crate::ClipboardReader>) {
        self.clipboard = reader;
    }

    /// Supply a hook that starts a clipboard read as soon as the typed text
    /// can only be completed into a `{{clipboard}}` snippet, so the render
    /// does not wait for `wl-paste`. Called at most once per typed prefix,
    /// and only when the clipboard variable is enabled and allowed.
    pub fn set_clipboard_prefetch(&mut self, prefetch: Option<crate::ClipboardPrefetch>) {
        self.clipboard_prefetch = prefetch;
    }

    /// The registered prefetch hook, for carrying it across a reload.
    pub fn clipboard_prefetch(&self) -> Option<crate::ClipboardPrefetch> {
        self.clipboard_prefetch.clone()
    }

    /// Fire the prefetch hook when a clipboard trigger is being typed.
    pub(crate) fn maybe_prefetch_clipboard(&mut self) {
        let (Some(prefetch), Some(triggers)) = (&self.clipboard_prefetch, &self.clipboard_triggers)
        else {
            return;
        };
        let enforcement = self.config.organization.effective_enforcement_policy();
        if !self.config.settings.allow_clipboard
            || enforcement.disable_clipboard
            || self.clipboard.is_none()
        {
            return;
        }
        if triggers.prefix_pending(&self.buffer) {
            if !self.clipboard_prefetch_armed {
                self.clipboard_prefetch_armed = true;
                (prefetch.0)();
            }
        } else {
            self.clipboard_prefetch_armed = false;
        }
    }

    /// The registered clipboard reader, for carrying it across a reload.
    pub fn clipboard_reader(&self) -> Option<crate::ClipboardReader> {
        self.clipboard.clone()
    }

    /// The context a snippet renders with right now.
    pub(crate) fn template_context(&self) -> crate::TemplateContext {
        let enforcement = self.config.organization.effective_enforcement_policy();
        crate::TemplateContext {
            env: std::sync::Arc::clone(&self.template_base.env),
            snippets: std::sync::Arc::clone(&self.template_base.snippets),
            clipboard: self
                .clipboard
                .clone()
                .filter(|_| self.config.settings.allow_clipboard && !enforcement.disable_clipboard),
            ..crate::TemplateContext::system()
        }
    }

    /// Register a wakeup for asynchronous expansion completions. Applies to
    /// workers that are already running as well as ones started later.
    pub fn set_completion_notifier(&mut self, notifier: Option<CompletionNotifier>) {
        match self.completion_notifier.write() {
            Ok(mut guard) => *guard = notifier,
            Err(poisoned) => *poisoned.into_inner() = notifier,
        }
    }

    /// The registered completion wakeup, for carrying it across a reload.
    pub fn completion_notifier(&self) -> Option<CompletionNotifier> {
        match self.completion_notifier.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Run command-backed expansions and hotkey actions on separate bounded
    /// worker queues. A slow expansion command cannot delay an urgent hotkey.
    /// This is enabled by the daemon; CLI and GUI previews remain synchronous
    /// so an explicit preview call can return its result directly.
    pub fn enable_async_commands(&mut self) -> bool {
        if self.async_commands.is_some() {
            return true;
        }
        let Some(runtime) = AsyncCommandRuntime::start(
            Arc::clone(&self.shared_input_generation),
            Arc::clone(&self.completion_notifier),
            Arc::clone(&self.expansion_metrics),
            Arc::clone(&self.hotkey_metrics),
        ) else {
            return false;
        };
        self.async_commands = Some(runtime);
        true
    }

    /// Return completed command expansions that are still safe to apply.
    /// Any intervening input, focus, pause, or window event advances the
    /// generation and causes late output to be discarded rather than erasing
    /// text at a cursor that may have moved.
    pub fn drain_completed_commands(&mut self) -> Vec<ExpansionResult> {
        let Some(runtime) = self.async_commands.as_ref() else {
            return Vec::new();
        };
        let completions: Vec<_> = runtime.receiver.try_iter().collect();
        let mut results = Vec::new();
        for completion in completions {
            if let Some(origin) = &completion.form {
                self.form_active = false;
                let Ok(output) = completion.output else {
                    self.restore_deferred_match(&completion.result.matched_text);
                    continue;
                };
                // Apply only where the form was opened: focus must be back in
                // that exact toplevel, not merely another window of the same
                // application. Backends without a strong window ID never
                // start a form job (see queue_form).
                let returned_to_origin = self.normalized_window.as_ref().is_some_and(|window| {
                    window.app_id == origin.app_id
                        && window.instance_id.as_deref() == Some(origin.instance_id.as_str())
                });
                let limit = self.config.organization.max_replacement_size;
                if self.sensitive_focus
                    || self.user_paused
                    || !returned_to_origin
                    || (limit > 0 && output.len() > limit)
                    || (completion.additional_max_size > 0
                        && output.len() > completion.additional_max_size)
                {
                    self.restore_deferred_match(&completion.result.matched_text);
                    continue;
                }
                let mut result = completion.result;
                result.insert = output;
                results.push(result);
                continue;
            }
            let output = match completion.output {
                Ok(output) => output,
                Err(_) => {
                    self.restore_deferred_match(&completion.result.matched_text);
                    continue;
                }
            };

            // Postflight policy: the output must still belong to the current
            // input generation, fit the configured limit, and the session must
            // not have become paused or sensitive while the command ran. This
            // used to rebuild a whole `MatchPlan` from the configuration --
            // deep-copying the trigger, the replacement, and the command on
            // every completion -- although the check reads none of those.
            if !self.postflight_allows(completion.generation, &output) {
                self.restore_deferred_match(&completion.result.matched_text);
                continue;
            }
            let validated_output = output;
            if completion.additional_max_size > 0
                && validated_output.len() > completion.additional_max_size
            {
                self.restore_deferred_match(&completion.result.matched_text);
                continue;
            }

            // Update cache before case propagation (cache stores original command output)
            if completion.cache_ms > 0 {
                self.command_cache[completion.config_index] = Some(CommandCacheEntry {
                    expires_at: Instant::now() + Duration::from_millis(completion.cache_ms),
                    value: validated_output.clone(),
                });
            }

            // Apply case propagation
            let final_output = if self.config.expansion[completion.config_index].propagate_case {
                matching::apply_case_style(&completion.result.matched_text, &validated_output)
            } else {
                validated_output
            };

            // Build final result
            let mut result = completion.result;
            result.insert = final_output;
            results.push(result);
        }
        results
    }

    /// Complete a deferred match after caller-side preflight policy approval.
    /// Cache and case state are prepared here; undo state is committed only
    /// after the caller successfully injects the returned result.
    /// `additional_max_size` is a separately loaded administrator limit;
    /// zero means that only the limit carried by the engine is applied.
    fn execute_pending_inline(
        &mut self,
        pending: PendingExpansionResult,
        additional_max_size: usize,
    ) -> Result<ExpansionResult, CommandError> {
        // Static results are completed immediately after their input event has
        // been parsed, so a later scalar in that same event may have advanced
        // the per-scalar generation. Command-backed results must still match
        // the current generation because they can complete asynchronously.
        if (pending.command.is_some() && pending.generation != self.input_generation)
            || self.user_paused
            || self.sensitive_focus
        {
            return Err(CommandError::StaleInput);
        }
        if pending
            .command
            .as_ref()
            .is_some_and(|command| self.command_execution_disabled(command))
        {
            return Err(CommandError::PolicyBlocked);
        }

        let command_backed = pending.command.is_some();
        let output = if let Some(cached) = pending.cached_output {
            cached
        } else if let Some(command) = &pending.command {
            run_command(command)?
        } else {
            pending.template_text
        };

        for limit in [pending.max_replacement_size, additional_max_size] {
            if limit > 0 && output.len() > limit {
                return Err(CommandError::PolicyOutputTooLarge {
                    size: output.len(),
                    limit,
                });
            }
        }

        if command_backed && pending.cache_ms > 0 {
            self.command_cache[pending.config_index] = Some(CommandCacheEntry {
                expires_at: Instant::now() + Duration::from_millis(pending.cache_ms),
                value: output.clone(),
            });
        }

        let insert = if pending.propagate_case {
            matching::apply_case_style(&pending.matched_text, &output)
        } else {
            output
        };
        Ok(ExpansionResult {
            snippet_id: pending.snippet_id,
            trigger: pending.trigger,
            matched_text: pending.matched_text,
            insert,
            cursor_offset: pending.cursor_offset,
            reinsert_after: pending.reinsert_after,
            command_backed,
            undoable: pending.undoable,
        })
    }

    /// Complete static/cache-hit matches immediately and queue uncached
    /// commands on the bounded expansion worker. Command execution never
    /// falls back to the caller thread when workers are unavailable.
    pub fn dispatch_pending_with_policy(
        &mut self,
        pending: PendingExpansionResult,
        additional_max_size: usize,
    ) -> Result<PendingExpansionDispatch, CommandError> {
        let matched_text = pending.matched_text.clone();
        if (pending.command.is_some() && pending.generation != self.input_generation)
            || self.user_paused
            || self.sensitive_focus
        {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::StaleInput);
        }
        if let Some(fields) = pending.form.clone() {
            return self.queue_form(pending, fields, additional_max_size);
        }
        if pending
            .command
            .as_ref()
            .is_some_and(|command| self.command_execution_disabled(command))
        {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::PolicyBlocked);
        }

        let Some(command) = pending.command.as_ref() else {
            return match self.execute_pending_inline(pending, additional_max_size) {
                Ok(result) => Ok(PendingExpansionDispatch::Ready(result)),
                Err(error) => {
                    self.restore_deferred_match(&matched_text);
                    Err(error)
                }
            };
        };
        if pending.cached_output.is_some() {
            return match self.execute_pending_inline(pending, additional_max_size) {
                Ok(result) => Ok(PendingExpansionDispatch::Ready(result)),
                Err(error) => {
                    self.restore_deferred_match(&matched_text);
                    Err(error)
                }
            };
        }

        let Some(runtime) = self.async_commands.as_ref() else {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::WorkerUnavailable);
        };
        let result = ExpansionResult {
            snippet_id: pending.snippet_id.clone(),
            trigger: pending.trigger,
            matched_text: pending.matched_text,
            insert: String::new(),
            cursor_offset: pending.cursor_offset,
            reinsert_after: pending.reinsert_after,
            command_backed: true,
            undoable: pending.undoable,
        };
        let job = AsyncCommandJob::Expansion {
            config_index: pending.config_index,
            generation: pending.generation,
            additional_max_size,
            command: command.clone(),
            result,
        };
        if let Err(error) = runtime.try_send_command(job) {
            self.restore_deferred_match(&matched_text);
            return Err(match error {
                QueueSendError::Full => CommandError::QueueFull,
                QueueSendError::Disconnected => CommandError::WorkerUnavailable,
            });
        }
        Ok(PendingExpansionDispatch::Queued)
    }

    fn queue_form(
        &mut self,
        pending: PendingExpansionResult,
        fields: Arc<Vec<crate::FormField>>,
        additional_max_size: usize,
    ) -> Result<PendingExpansionDispatch, CommandError> {
        let matched_text = pending.matched_text.clone();
        if self.form_active || pending.generation != self.input_generation {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::StaleInput);
        }
        let origin = self.normalized_window.as_ref().and_then(|window| {
            let instance_id = window.instance_id.as_ref()?;
            (!instance_id.is_empty() && instance_id.len() <= MAX_WINDOW_INSTANCE_ID_BYTES).then(
                || FormOrigin {
                    app_id: window.app_id.clone(),
                    instance_id: instance_id.clone(),
                },
            )
        });
        let Some(origin) = origin else {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::WindowIdentityUnavailable);
        };
        let Some(runtime) = self.async_commands.as_ref() else {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::WorkerUnavailable);
        };
        let job = AsyncCommandJob::Form {
            config_index: pending.config_index,
            additional_max_size,
            template: pending.template_text,
            fields,
            context: self.template_context(),
            title: pending.trigger.clone(),
            origin,
            result: ExpansionResult {
                snippet_id: pending.snippet_id,
                trigger: pending.trigger,
                matched_text: pending.matched_text,
                insert: String::new(),
                cursor_offset: None,
                reinsert_after: pending.reinsert_after,
                command_backed: false,
                undoable: pending.undoable,
            },
        };
        if let Err(error) = runtime.try_send_command(job) {
            self.restore_deferred_match(&matched_text);
            return Err(match error {
                QueueSendError::Full => CommandError::QueueFull,
                QueueSendError::Disconnected => CommandError::WorkerUnavailable,
            });
        }
        self.form_active = true;
        Ok(PendingExpansionDispatch::Queued)
    }

    /// Restore a deferred trigger when its result will not be injected. The
    /// trigger is put back before any later input so matcher order remains the
    /// same as the non-exclusive application's input stream.
    pub fn restore_deferred_match(&mut self, matched_text: &str) {
        if self.take_deferred_reservation(matched_text).is_some() {
            self.restore_buffered(matched_text);
        }
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Remove the reservation represented by a result. Evdev may absorb one
    /// delimiter into `matched_text` after the reservation was created, so a
    /// result can be the reserved trigger plus exactly one scalar.
    fn take_deferred_reservation(&mut self, matched_text: &str) -> Option<String> {
        let index = self.deferred_matches.iter().position(|reserved| {
            reserved == matched_text
                || matched_text
                    .strip_prefix(reserved)
                    .is_some_and(|suffix| suffix.chars().count() == 1)
        })?;
        Some(self.deferred_matches.remove(index))
    }

    fn release_deferred_match_for_result(&mut self, result: &ExpansionResult) {
        self.take_deferred_reservation(&result.matched_text);
    }

    fn restore_deferred_matches(&mut self) {
        let reservations = std::mem::take(&mut self.deferred_matches);
        for matched_text in reservations {
            self.restore_buffered(&matched_text);
        }
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Append restored text through the same bounded path as ordinary input.
    /// Deferred work may restore several reservations at once, so bypassing
    /// this helper can temporarily violate the matcher's rolling-buffer cap.
    fn restore_buffered(&mut self, text: &str) {
        for character in text.chars() {
            self.push_buffered(character);
        }
    }

    /// Append one character while maintaining the configured rolling-buffer
    /// bound and its word-boundary truncation metadata.
    fn push_buffered(&mut self, character: char) {
        self.buffer.push_back(character);
        while self.buffer.len() > self.max_buffer_chars {
            self.buffer.pop_front();
            self.buffer_truncated = true;
        }
    }

    /// Unified commit: apply all post-execution logic to create final ExpansionResult.
    /// This is the single path for committing any expansion (static or command-backed).
    /// Handles case propagation, undo state, and result metadata.
    /// Takes the plan by value: it is always the caller's last use of it, and
    /// the trigger and matched text move straight into the result instead of
    /// being copied out of a plan that is dropped on the next line.
    fn commit_expansion(&mut self, plan: MatchPlan, insert: String) -> ExpansionResult {
        let mut final_insert = insert;

        // Apply case propagation if configured (applies to all expansion types)
        if plan.propagate_case {
            final_insert = matching::apply_case_style(&plan.matched_text, &final_insert);
        }

        let reinsert_after = plan.terminating_char.filter(|_| self.reinsert_terminators);

        // Save undo state (only if no cursor offset - cursor marker expansions don't support undo)
        // Skip undo for async commands since they return empty immediately
        if plan.cursor_offset.is_none()
            && (!plan.is_command_backed() || self.async_commands.is_none())
        {
            self.last_expansion = Some(transaction::transaction_texts(
                &plan.matched_text,
                &final_insert,
                reinsert_after,
            ));
        }

        let command_backed = plan.is_command_backed();
        ExpansionResult {
            snippet_id: plan.snippet_id,
            command_backed,
            trigger: plan.trigger_config,
            matched_text: plan.matched_text,
            insert: final_insert,
            cursor_offset: plan.cursor_offset,
            reinsert_after,
            undoable: true,
        }
    }

    fn take_match(
        &mut self,
        config_index: usize,
        length: usize,
        terminating_char: Option<char>,
    ) -> Option<ExpansionResult> {
        // Generate match plan with full context
        let mut plan = self.take_match_plan(config_index, length, terminating_char)?;
        // Forms need the deferred, asynchronous path; a synchronous match
        // must never type the raw field markers.
        if plan.form.is_some() {
            return None;
        }

        // Apply preflight policy
        if !self.preflight_allows(&plan) {
            return None;
        }

        let expansion = &self.config.expansion[config_index];

        // Check cache before deciding on async vs sync
        let cached_command = expansion.command.as_ref().and_then(|command| {
            (command.cache_ms > 0)
                .then(|| self.command_cache[config_index].as_ref())
                .flatten()
                .filter(|entry| entry.expires_at > Instant::now())
                .map(|entry| entry.value.clone())
        });

        // Cached command: use cache
        if let Some(cached_value) = cached_command {
            for _ in 0..length {
                self.buffer.pop_back();
            }
            // Commit with cached value (already cached, no re-caching needed)
            return Some(self.commit_expansion(plan, cached_value));
        }

        // Async command: queue and return empty
        if let Some(runtime) = self.async_commands.as_ref() {
            // Binding the command here rather than testing `is_some()` and
            // unwrapping below keeps the queueing path free of a panic that
            // only a future edit could ever trigger.
            if let Some(command) = expansion.command.as_ref() {
                let reinsert_after = plan.terminating_char.filter(|_| self.reinsert_terminators);
                let job = AsyncCommandJob::Expansion {
                    config_index,
                    generation: self.input_generation,
                    additional_max_size: 0,
                    command: command.clone(),
                    result: ExpansionResult {
                        snippet_id: plan.snippet_id,
                        trigger: plan.trigger_config,
                        matched_text: plan.matched_text,
                        insert: String::new(),
                        cursor_offset: None,
                        reinsert_after,
                        command_backed: true,
                        undoable: true,
                    },
                };
                if runtime.try_send_command(job).is_err() {
                    return None;
                }
                // Do not consume the trigger until the worker has accepted
                for _ in 0..length {
                    self.buffer.pop_back();
                }
                self.last_expansion = None;
                return None;
            }
        }

        // Sync fallback: static templates were rendered as part of the plan;
        // command output still executes here (or uses its cache above).
        // The rendered template moves out of the plan -- `commit_expansion`
        // never reads it back, so copying it here allocated a second full
        // replacement string on every plain-text expansion.
        let insert = if expansion.command.is_some() {
            self.render_expansion(config_index).ok()?
        } else {
            std::mem::take(&mut plan.replacement_text)
        };
        for _ in 0..length {
            self.buffer.pop_back();
        }
        Some(self.commit_expansion(plan, insert))
    }

    /// Deferred execution variant: returns pending results without executing commands.
    /// Caller must check policy and complete through the
    /// engine so cache and case propagation are prepared. Undo state is
    /// committed by the caller only after successful injection.
    fn take_match_deferred(
        &mut self,
        config_index: usize,
        length: usize,
        terminating_char: Option<char>,
    ) -> Option<PendingExpansionResult> {
        // Generate match plan with full context
        let plan = self.take_match_plan(config_index, length, terminating_char)?;

        // Policy is checked by the caller before deferred completion.

        // Static templates were rendered once while creating the plan. For a
        // command-backed match this field is unused until command completion.
        let template_text = plan.replacement_text.clone();

        // Consume buffer
        for _ in 0..length {
            self.buffer.pop_back();
        }
        self.deferred_matches.push(plan.matched_text.clone());

        Some(PendingExpansionResult {
            snippet_id: plan.snippet_id,
            trigger: plan.trigger_config,
            matched_text: plan.matched_text,
            template_text,
            cursor_offset: plan.cursor_offset,
            reinsert_after: plan.terminating_char.filter(|_| self.reinsert_terminators),
            max_replacement_size: self.config.organization.max_replacement_size,
            config_index,
            generation: plan.generation,
            propagate_case: plan.propagate_case,
            cache_ms: plan.command.as_ref().map_or(0, |command| command.cache_ms),
            cached_output: plan.command.as_ref().and_then(|command| {
                (command.cache_ms > 0)
                    .then(|| self.command_cache[config_index].as_ref())
                    .flatten()
                    .filter(|entry| entry.expires_at > Instant::now())
                    .map(|entry| entry.value.clone())
            }),
            undoable: true,
            command: plan.command.as_ref().map(|c| (**c).clone()),
            form: plan.form.clone(),
        })
    }

    /// Execute a command-backed expansion, using the short-lived cache when
    /// configured. Static templates are rendered while creating `MatchPlan`.
    fn render_expansion(&mut self, config_index: usize) -> Result<String, ()> {
        let expansion = &self.config.expansion[config_index];
        let Some(command) = &expansion.command else {
            return Err(());
        };
        if command.cache_ms > 0 {
            if let Some(entry) = self.command_cache[config_index].as_ref() {
                if entry.expires_at > Instant::now() {
                    return Ok(entry.value.clone());
                }
            }
        }
        let value = run_command(command).map_err(|_| ())?;
        if command.cache_ms > 0 {
            self.command_cache[config_index] = Some(CommandCacheEntry {
                expires_at: Instant::now() + Duration::from_millis(command.cache_ms),
                value: value.clone(),
            });
        }
        Ok(value)
    }

    /// Resets the matcher buffer to a known boundary. Always use this
    /// instead of `self.buffer.clear()` directly: it also clears
    /// `buffer_truncated`, since a fresh boundary means whatever preceded it
    /// is no longer relevant to word-boundary checks.
    fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.buffer_truncated = false;
        self.clipboard_prefetch_armed = false;
    }
}

#[cfg(test)]
mod tests;
