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
mod preview_sandbox;
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
pub use transaction::{
    route_limit, RouteLimit, TransactionOutcome, TransactionPhase, TransactionReport,
};

use cache::{ClipboardTriggers, CommandCacheEntry};
use expansion::MatchScratch;
use matching::GlobPattern;
use state::NormalizedWindowContext;

use async_runtime::{AsyncCommandJob, AsyncCommandRuntime, FormOrigin};
pub use command_runtime::{run_command, run_command_cancellable};
use command_runtime::{CommandMetricsState, QueueSendError};
pub use preview_sandbox::{command_preview_sandboxed, run_command_preview};

use crate::{
    AppFilter, CommandConfig, CommandEnvironment, Config, ConfigError, HotkeyConfig,
    InjectorCapabilities, KeyChord, Matcher,
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroize;

pub(crate) use crate::limits::{
    MAX_COMMAND_OUTPUT_BYTES, MAX_RESULTS_PER_EVENT, MAX_RESULT_BYTES_PER_EVENT,
};
const MINIMAL_COMMAND_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const ASYNC_COMMAND_QUEUE_CAPACITY: usize = 16;
const ASYNC_COMMAND_WORKER_COUNT: usize = 4;
const COMMAND_FAILURE_HISTORY_LIMIT: usize = 16;

struct DeferredMatchReservation {
    matched_text: String,
    command_backed: bool,
}

/// Maximum accepted size for opaque backend-provided toplevel identities.
pub const MAX_WINDOW_INSTANCE_ID_BYTES: usize = 192;

pub struct ExpansionEngine {
    config: Arc<Config>,
    /// The validated source snapshot remains alive for this engine's entire
    /// lifetime, so runtime matching and diagnostics share its interpretation.
    validated: Arc<crate::ValidatedConfig>,
    /// Runtime-only restrictions supplied by the host (for example command
    /// worker availability) are kept outside the immutable source snapshot.
    runtime_policy: crate::OrganizationPolicy,
    /// Compiled app-id globs corresponding positionally to `app_filters`.
    app_filter_globs: Vec<Vec<Option<GlobPattern>>>,
    matcher: Matcher,
    /// Reused temporary storage for NFC normalization and raw-offset mapping
    /// of decomposed input suffixes.
    match_scratch: MatchScratch,
    buffer: VecDeque<char>,
    /// UTF-8 length of `buffer`, maintained with the character deque so
    /// grapheme backspace can address bounded suffix chunks without rescanning
    /// or materializing the complete history.
    buffered_bytes: usize,
    /// Number of non-ASCII scalars in `buffer`. An all-ASCII buffer is
    /// necessarily NFC, so matching can skip rescanning it on every input.
    non_ascii_buffer_chars: usize,
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
    command_cache_expirations: BinaryHeap<Reverse<(Instant, usize, u64)>>,
    next_command_cache_generation: u64,
    current_window: Option<WindowContext>,
    normalized_window: Option<NormalizedWindowContext>,
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
    deferred_matches: Vec<DeferredMatchReservation>,
    /// How many trailing buffer characters were restored from deferred
    /// reservations. They already matched once, so they are context for
    /// later input but must not complete a trigger again on their own.
    restored_reservation_chars: usize,
    /// Bounded content-free categories for command completions that never
    /// reached output. Hosts drain these into their diagnostics surfaces.
    command_failures: VecDeque<CommandErrorKind>,
    input_generation: u64,
    shared_input_generation: Arc<AtomicU64>,
    async_commands: Option<AsyncCommandRuntime>,
    /// Called by expansion workers after each completion is queued, so a host
    /// without its own event loop can drain results on wakeup instead of
    /// polling. Shared with running workers so it can be set at any time.
    completion_notifier: Arc<RwLock<Option<CompletionNotifier>>>,
    expansion_metrics: Arc<CommandMetricsState>,
    hotkey_metrics: Arc<CommandMetricsState>,
    /// Whether a terminating character is carried through the replacement:
    /// erased together with the trigger and typed again after the insert.
    /// Every live source has already delivered the delimiter to the
    /// application by the time it matches (the input method commits each
    /// typed character; evdev is non-exclusive), so hosts keep this enabled.
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
    usage_events: VecDeque<(u64, crate::UsageEvent)>,
    /// Events evicted when the host does not collect usage events promptly.
    usage_events_dropped: u64,
    /// A snippet form is open; capture is suspended until it completes.
    form_active: bool,
    /// What the host's current output route guarantees about erasing the
    /// trigger. A form completes after focus has left and returned, so the
    /// caret may have moved (a mouse click is invisible to every capture
    /// backend); forms are opened and applied only when the route refuses to
    /// replace unless the trigger is still at the cursor.
    form_replacement_guarantee: crate::ReplacementGuarantee,
}

impl ExpansionEngine {
    /// The immutable validated source and derived metadata used by this
    /// engine. Runtime-only host restrictions are exposed separately through
    /// policy diagnostics and do not mutate the snapshot.
    pub fn validated_config(&self) -> &Arc<crate::ValidatedConfig> {
        &self.validated
    }

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
        let validated = Arc::new(config.validate_snapshot_with_snippets(snippets)?);
        Ok(Self::from_validated(validated))
    }

    /// Construct directly from a snapshot that has already passed the full
    /// validator. Reload and preview paths can carry the same object through
    /// validation and engine construction without reparsing derived data.
    pub fn from_validated(validated: Arc<crate::ValidatedConfig>) -> Self {
        let config = Arc::clone(&validated.config);
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
        let (clipboard, others): (Vec<_>, Vec<_>) =
            validated.effective_triggers.iter().partition(|(index, _)| {
                let expansion = &config.expansion[*index];
                expansion.command.is_none()
                    && validated.template_variables[*index]
                        .iter()
                        .any(|name| name == "clipboard")
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
        // Snapshot validation caps effective triggers far below the matcher's
        // representable index count. Keep the public constructor fallible for
        // arbitrary callers; this internal boundary consumes validated data.
        let matcher = Matcher::try_new(
            validated
                .effective_triggers
                .iter()
                .map(|(_, trigger)| trigger.as_str()),
        )
        .expect("validated effective-trigger limit fits matcher indices");
        let app_filter_globs = validated
            .app_filters
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
        let template_base =
            config.template_context_with_snippets(None, Arc::clone(&validated.snippets));
        let max_buffer_chars = config.settings.max_buffer_chars;
        let command_cache = std::iter::repeat_with(|| None)
            .take(config.expansion.len())
            .collect();
        Self {
            config,
            validated: Arc::clone(&validated),
            runtime_policy: validated.resolved_policy.clone(),
            app_filter_globs,
            matcher,
            match_scratch: MatchScratch::default(),
            buffer: VecDeque::new(),
            buffered_bytes: 0,
            non_ascii_buffer_chars: 0,
            max_buffer_chars,
            buffer_truncated: false,
            user_paused: false,
            sensitive_focus: false,
            composition_active: false,
            direct_commands_disabled: false,
            command_cache,
            command_cache_expirations: BinaryHeap::new(),
            next_command_cache_generation: 0,
            current_window: None,
            normalized_window: None,
            last_expansion: None,
            deferred_matches: Vec::new(),
            restored_reservation_chars: 0,
            command_failures: VecDeque::with_capacity(COMMAND_FAILURE_HISTORY_LIMIT),
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
            usage_events_dropped: 0,
            form_active: false,
            form_replacement_guarantee: crate::ReplacementGuarantee::Unsupported,
        }
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
        let mut candidate = (*self.config).clone();
        candidate.apply_administrator_policy(policy)?;
        let validated = Arc::new(
            candidate
                .validate_snapshot_with_snippets(Some(Arc::clone(&self.validated.snippets)))?,
        );
        self.config = Arc::clone(&validated.config);
        self.runtime_policy = validated.resolved_policy.clone();
        self.validated = validated;
        Ok(())
    }

    /// Configure whether a match's terminating delimiter is erased with the
    /// trigger and reinserted after the replacement. Enabled by default; only
    /// a host whose source withholds the delimiter from the application
    /// should disable it.
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
            || self.config.settings.clipboard_mode != crate::ClipboardMode::Prefetch
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

    /// Render expansion `index` from the template compiled at validation.
    pub(crate) fn render_replacement(
        &self,
        index: usize,
    ) -> Result<(String, Option<usize>), crate::TemplateError> {
        match self
            .validated
            .compiled_templates
            .get(index)
            .and_then(Option::as_ref)
        {
            Some(compiled) => compiled.render_with_cursor(&self.template_context()),
            None => crate::render_template_with_cursor(
                &self.config.expansion[index].replacement,
                &self.template_context(),
            ),
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
                let output = match completion.output {
                    Ok(output) => output,
                    Err(error) => {
                        self.remember_command_failure(error.kind());
                        self.restore_failed_command_match(&completion.result.matched_text);
                        continue;
                    }
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
                let target_verifiable = self.form_target_verifiable();
                let exceeds_limit = (limit > 0 && output.len() > limit)
                    || (completion.additional_max_size > 0
                        && output.len() > completion.additional_max_size);
                let failure_kind = if self.sensitive_focus || self.user_paused {
                    Some(CommandErrorKind::UnsafeContext)
                } else if !returned_to_origin || !target_verifiable {
                    Some(CommandErrorKind::WindowIdentity)
                } else if exceeds_limit {
                    Some(CommandErrorKind::OutputTooLarge)
                } else {
                    None
                };
                if let Some(kind) = failure_kind {
                    self.remember_command_failure(kind);
                    self.restore_failed_command_match(&completion.result.matched_text);
                    continue;
                }
                let mut result = completion.result;
                result.insert = output;
                results.push(result);
                continue;
            }
            let output = match completion.output {
                Ok(output) => output,
                Err(error) => {
                    self.remember_command_failure(error.kind());
                    self.restore_failed_command_match(&completion.result.matched_text);
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
                let kind = if self.config.organization.max_replacement_size > 0
                    && output.len() > self.config.organization.max_replacement_size
                {
                    CommandErrorKind::OutputTooLarge
                } else if self.sensitive_focus || self.user_paused {
                    CommandErrorKind::UnsafeContext
                } else {
                    CommandErrorKind::StaleInput
                };
                self.remember_command_failure(kind);
                self.restore_failed_command_match(&completion.result.matched_text);
                continue;
            }
            let validated_output = output;
            if completion.additional_max_size > 0
                && validated_output.len() > completion.additional_max_size
            {
                self.remember_command_failure(CommandErrorKind::OutputTooLarge);
                self.restore_failed_command_match(&completion.result.matched_text);
                continue;
            }

            // Update cache before case propagation (cache stores original command output)
            if completion.cache_ms > 0 {
                self.cache_command_output(
                    completion.config_index,
                    completion.cache_ms,
                    validated_output.clone(),
                );
            }

            // Apply case propagation
            let final_output = if self.config.expansion[completion.config_index].propagate_case {
                matching::apply_case_style(
                    &completion.result.matched_text,
                    std::borrow::Cow::Owned(validated_output),
                )
                .into_owned()
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

    /// Drain bounded, content-free reasons why completed command expansions
    /// were discarded before reaching the output backend.
    pub fn drain_command_failures(&mut self) -> Vec<CommandErrorKind> {
        self.command_failures.drain(..).collect()
    }

    fn remember_command_failure(&mut self, kind: CommandErrorKind) {
        if self.command_failures.len() == COMMAND_FAILURE_HISTORY_LIMIT {
            self.command_failures.pop_front();
        }
        self.command_failures.push_back(kind);
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
            || (pending.command.is_some() && pending.captured_window != self.current_window)
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
            self.cache_command_output(pending.config_index, pending.cache_ms, output.clone());
        }

        let insert = if pending.propagate_case {
            matching::apply_case_style(&pending.matched_text, std::borrow::Cow::Owned(output))
                .into_owned()
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
            folded_suffix_chars: 0,
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
            || (pending.command.is_some() && pending.captured_window != self.current_window)
        {
            if pending.command.is_some() {
                self.restore_failed_command_match(&matched_text);
            } else {
                self.restore_deferred_match(&matched_text);
            }
            return Err(CommandError::StaleInput);
        }
        if pending
            .command
            .as_ref()
            .is_some_and(|command| self.command_execution_disabled(command))
        {
            self.restore_failed_command_match(&matched_text);
            return Err(CommandError::PolicyBlocked);
        }
        if let Some(fields) = pending.form.clone() {
            return self.queue_form(pending, fields, additional_max_size);
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
            folded_suffix_chars: 0,
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
                QueueSendError::Disconnected | QueueSendError::Unavailable => {
                    CommandError::WorkerUnavailable
                }
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
        if !self.form_target_verifiable() {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::FormTargetUnverifiable);
        }
        let Some(runtime) = self.async_commands.as_ref() else {
            self.restore_deferred_match(&matched_text);
            return Err(CommandError::WorkerUnavailable);
        };
        let command_backed = pending.command.is_some();
        let job = AsyncCommandJob::Form {
            config_index: pending.config_index,
            additional_max_size,
            template: pending.form_template.unwrap_or_else(|| Arc::from("")),
            fields,
            command: pending.command.clone(),
            context: self.template_context(),
            title: pending.trigger.to_string(),
            origin,
            result: ExpansionResult {
                snippet_id: pending.snippet_id,
                trigger: pending.trigger,
                matched_text: pending.matched_text,
                insert: String::new(),
                cursor_offset: None,
                reinsert_after: pending.reinsert_after,
                command_backed,
                undoable: pending.undoable,
                folded_suffix_chars: 0,
            },
        };
        if let Err(error) = runtime.try_send_command(job) {
            self.restore_deferred_match(&matched_text);
            return Err(match error {
                QueueSendError::Full => CommandError::QueueFull,
                QueueSendError::Disconnected | QueueSendError::Unavailable => {
                    CommandError::WorkerUnavailable
                }
            });
        }
        self.form_active = true;
        Ok(PendingExpansionDispatch::Queued)
    }

    /// Declare the current output route's replacement guarantee. Hosts must
    /// keep this current; the default refuses every form.
    pub fn set_form_replacement_guarantee(&mut self, guarantee: crate::ReplacementGuarantee) {
        self.form_replacement_guarantee = guarantee;
    }

    pub fn form_replacement_guarantee(&self) -> crate::ReplacementGuarantee {
        self.form_replacement_guarantee
    }

    fn form_target_verifiable(&self) -> bool {
        self.form_replacement_guarantee >= crate::ReplacementGuarantee::VerifiedSurroundingText
    }

    /// Whether a deferred result still holds its trigger reservation: it was
    /// matched and dispatched but neither injected nor restored yet, as while
    /// its command runs on the expansion worker.
    pub fn has_deferred_reservations(&self) -> bool {
        !self.deferred_matches.is_empty()
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
    fn take_deferred_reservation(&mut self, matched_text: &str) -> Option<()> {
        let index = self.deferred_matches.iter().position(|reservation| {
            reservation.matched_text == matched_text
                || matched_text
                    .strip_prefix(&reservation.matched_text)
                    .is_some_and(|suffix| suffix.chars().count() == 1)
        })?;
        let mut reservation = self.deferred_matches.remove(index);
        reservation.matched_text.zeroize();
        Some(())
    }

    /// The text this result reserved when it was matched, before a host folded
    /// any later-typed characters into it.
    fn reserved_text(result: &ExpansionResult) -> &str {
        let end = result
            .matched_text
            .char_indices()
            .rev()
            .nth(result.folded_suffix_chars.saturating_sub(1))
            .filter(|_| result.folded_suffix_chars > 0)
            .map_or(result.matched_text.len(), |(index, _)| index);
        &result.matched_text[..end]
    }

    fn release_deferred_match_for_result(&mut self, result: &ExpansionResult) {
        self.take_deferred_reservation(Self::reserved_text(result));
    }

    /// Restore a deferred result that will not be injected, including any
    /// characters a host folded into it: they are already in the target.
    pub fn restore_deferred_result(&mut self, result: &ExpansionResult) {
        if self
            .take_deferred_reservation(Self::reserved_text(result))
            .is_some()
        {
            self.restore_buffered(&result.matched_text);
        }
    }

    fn restore_deferred_matches(&mut self) {
        let reservations = std::mem::take(&mut self.deferred_matches);
        for mut reservation in reservations {
            self.restore_buffered_as_context(&reservation.matched_text);
            reservation.matched_text.zeroize();
        }
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// A command reservation becomes stale as soon as later text arrives.
    /// Restore it before scanning that text so the buffer follows document
    /// order, and keep it context-only so the completed command cannot run a
    /// second time. Static deferred results remain reserved until their
    /// caller applies them.
    fn restore_stale_command_reservations(&mut self) {
        let mut retained = Vec::with_capacity(self.deferred_matches.len());
        for mut reservation in std::mem::take(&mut self.deferred_matches) {
            if reservation.command_backed {
                self.restore_buffered_as_context(&reservation.matched_text);
                reservation.matched_text.zeroize();
            } else {
                retained.push(reservation);
            }
        }
        self.deferred_matches = retained;
        debug_assert!(self.buffer.len() <= self.max_buffer_chars);
    }

    /// Whether a suffix of `length` characters lies entirely inside text that
    /// was restored from deferred reservations.
    fn suffix_is_restored_reservation(&self, length: usize) -> bool {
        length <= self.restored_reservation_chars.min(self.buffer.len())
    }

    /// Append restored text through the same bounded path as ordinary input.
    /// Deferred work may restore several reservations at once, so bypassing
    /// this helper can temporarily violate the matcher's rolling-buffer cap.
    fn restore_buffered(&mut self, text: &str) {
        for character in text.chars() {
            self.push_buffered(character);
        }
    }

    /// Restore a trigger that already matched as context only: the next
    /// character cannot complete it again. Used where a retry would be wrong
    /// -- the result went stale because later input arrived, or its command
    /// failed -- because matching again would rerun the command or erase the
    /// user's newer text. Known not-applied injections use the plain restore,
    /// so the next keystroke retries them.
    fn restore_buffered_as_context(&mut self, text: &str) {
        let restored = self.restored_reservation_chars;
        self.restore_buffered(text);
        self.restored_reservation_chars = restored
            .saturating_add(text.chars().count())
            .min(self.buffer.len());
    }

    /// Restore a discarded command completion's trigger as context only.
    fn restore_failed_command_match(&mut self, matched_text: &str) {
        if self.take_deferred_reservation(matched_text).is_some() {
            self.restore_buffered_as_context(matched_text);
        }
    }

    /// Append one character while maintaining the configured rolling-buffer
    /// bound and its word-boundary truncation metadata.
    fn push_buffered(&mut self, character: char) {
        self.restored_reservation_chars = 0;
        self.buffered_bytes = self.buffered_bytes.saturating_add(character.len_utf8());
        if !character.is_ascii() {
            self.non_ascii_buffer_chars += 1;
        }
        self.buffer.push_back(character);
        while self.buffer.len() > self.max_buffer_chars {
            let mut evicted =
                pop_front_zeroized(&mut self.buffer).expect("buffer length exceeded its cap");
            if !evicted.is_ascii() {
                self.non_ascii_buffer_chars -= 1;
            }
            self.buffered_bytes -= evicted.len_utf8();
            evicted.zeroize();
            self.buffer_truncated = true;
        }
    }

    fn pop_buffered_back(&mut self) {
        let mut removed = pop_back_zeroized(&mut self.buffer)
            .expect("grapheme cleanup exceeded the buffer length");
        if !removed.is_ascii() {
            self.non_ascii_buffer_chars -= 1;
        }
        self.buffered_bytes -= removed.len_utf8();
        removed.zeroize();
    }

    /// Unified commit: apply all post-execution logic to create final ExpansionResult.
    /// This is the single path for committing any expansion (static or command-backed).
    /// Handles case propagation, undo state, and result metadata.
    /// Takes the plan by value: it is always the caller's last use of it, and
    /// the trigger and matched text move straight into the result instead of
    /// being copied out of a plan that is dropped on the next line.
    fn commit_expansion(&mut self, plan: MatchPlan, insert: String) -> ExpansionResult {
        // Cow keeps this already-owned rendered String in place for the
        // common lower/mixed-case paths; transformed casing creates new text.
        let final_insert = if plan.propagate_case {
            matching::apply_case_style(&plan.matched_text, std::borrow::Cow::Owned(insert))
                .into_owned()
        } else {
            insert
        };

        let reinsert_after = plan.terminating_char.filter(|_| self.reinsert_terminators);

        // Save undo state (only if no cursor offset - cursor marker expansions don't support undo)
        // Skip undo for async commands since they return empty immediately
        if plan.cursor_offset.is_none()
            && (!plan.is_command_backed() || self.async_commands.is_none())
            && self.validated.undo_chord.is_some()
        {
            self.clear_undo_state();
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
            folded_suffix_chars: 0,
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

        // Check cache before deciding on async vs sync
        let cache_ms = self.config.expansion[config_index]
            .command
            .as_ref()
            .map_or(0, |command| command.cache_ms);
        let cached_command = (cache_ms > 0)
            .then(|| self.cached_command_output(config_index))
            .flatten();

        // Cached command: use cache
        if let Some(cached_value) = cached_command {
            self.consume_matched_text();
            // Commit with cached value (already cached, no re-caching needed)
            return Some(self.commit_expansion(plan, cached_value));
        }

        let expansion = &self.config.expansion[config_index];

        // Async command: queue and return empty
        if let Some(runtime) = self.async_commands.as_ref() {
            // Binding the command here rather than testing `is_some()` and
            // unwrapping below keeps the queueing path free of a panic that
            // only a future edit could ever trigger.
            if let Some(command) = plan.command.clone() {
                let reinsert_after = plan.terminating_char.filter(|_| self.reinsert_terminators);
                let job = AsyncCommandJob::Expansion {
                    config_index,
                    generation: self.input_generation,
                    additional_max_size: 0,
                    command,
                    result: ExpansionResult {
                        snippet_id: plan.snippet_id,
                        trigger: plan.trigger_config,
                        matched_text: plan.matched_text,
                        insert: String::new(),
                        cursor_offset: None,
                        reinsert_after,
                        command_backed: true,
                        undoable: true,
                        folded_suffix_chars: 0,
                    },
                };
                if let Err(error) = runtime.try_send_command(job) {
                    self.remember_command_failure(match error {
                        QueueSendError::Full => CommandErrorKind::QueueFull,
                        QueueSendError::Disconnected | QueueSendError::Unavailable => {
                            CommandErrorKind::WorkerUnavailable
                        }
                    });
                    return None;
                }
                // Do not consume the trigger until the worker has accepted
                self.consume_matched_text();
                self.clear_undo_state();
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
        self.consume_matched_text();
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
        let mut plan = self.take_match_plan(config_index, length, terminating_char)?;
        let cache_ms = plan.command.as_ref().map_or(0, |command| command.cache_ms);
        let cached_output = (cache_ms > 0)
            .then(|| self.cached_command_output(config_index))
            .flatten();

        // Policy is checked by the caller before deferred completion.

        // Static templates were rendered once while creating the plan. For a
        // command-backed match this field is unused until command completion.
        let template_text = if plan.command.is_some() || plan.form.is_some() {
            String::new()
        } else {
            std::mem::take(&mut plan.replacement_text)
        };

        // Consume buffer
        self.consume_matched_text();
        self.deferred_matches.push(DeferredMatchReservation {
            matched_text: plan.matched_text.clone(),
            command_backed: plan.command.is_some(),
        });

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
            // Only command-backed results complete late enough to need it.
            captured_window: plan
                .command
                .as_ref()
                .and_then(|_| self.current_window.clone()),
            propagate_case: plan.propagate_case,
            cache_ms,
            cached_output,
            undoable: true,
            command: plan.command.clone(),
            form_template: plan.form_template,
            form: plan.form,
        })
    }

    /// Execute a command-backed expansion, using the short-lived cache when
    /// configured. Static templates are rendered while creating `MatchPlan`.
    fn render_expansion(&mut self, config_index: usize) -> Result<String, ()> {
        let Some(command) = self.config.expansion[config_index].command.clone() else {
            return Err(());
        };
        if command.cache_ms > 0 {
            if let Some(value) = self.cached_command_output(config_index) {
                return Ok(value);
            }
        }
        let value = run_command(&command).map_err(|_| ())?;
        if command.cache_ms > 0 {
            self.cache_command_output(config_index, command.cache_ms, value.clone());
        }
        Ok(value)
    }

    /// Drop a matched trigger from the buffer together with everything typed
    /// before it. The replacement is not mirrored in the buffer, so that
    /// earlier text is no longer what precedes the cursor: kept, a Backspace
    /// into the replacement would pop it instead, and it could resurface to
    /// complete a trigger the document does not contain (and erase the
    /// user's text in its place). What precedes the cursor is now unknown,
    /// so a word-boundary trigger typed straight after the replacement does
    /// not match as if it began the input.
    fn consume_matched_text(&mut self) {
        self.clear_buffer();
        self.buffer_truncated = true;
    }

    /// Expire and wipe command output cache entries. The heap keeps cleanup
    /// proportional to entries actually expiring rather than scanning the
    /// entire snippet library on each keystroke.
    pub fn expire_command_cache(&mut self) {
        let now = Instant::now();
        while let Some(Reverse((expires_at, index, generation))) =
            self.command_cache_expirations.peek().copied()
        {
            if expires_at > now {
                break;
            }
            self.command_cache_expirations.pop();
            let expired_current_entry = self
                .command_cache
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|entry| entry.generation == generation && entry.expires_at <= now);
            if expired_current_entry {
                self.command_cache[index] = None;
            }
        }
    }

    fn cached_command_output(&mut self, index: usize) -> Option<String> {
        self.expire_command_cache();
        self.command_cache
            .get(index)
            .and_then(Option::as_ref)
            .map(|entry| entry.value.clone())
    }

    fn cache_command_output(&mut self, index: usize, ttl_ms: u64, value: String) {
        if ttl_ms == 0 || index >= self.command_cache.len() {
            return;
        }
        let expires_at = Instant::now() + Duration::from_millis(ttl_ms);
        self.next_command_cache_generation = self.next_command_cache_generation.wrapping_add(1);
        let generation = self.next_command_cache_generation;
        self.command_cache[index] = Some(CommandCacheEntry {
            expires_at,
            generation,
            value,
        });
        self.command_cache_expirations
            .push(Reverse((expires_at, index, generation)));

        // Replaced cache entries leave stale heap records until their old
        // deadlines. Compact on cache writes once those records exceed a
        // small multiple of the configured snippet count.
        let heap_limit = self.command_cache.len().saturating_mul(4).max(64);
        if self.command_cache_expirations.len() > heap_limit {
            self.command_cache_expirations.clear();
            for (index, entry) in self.command_cache.iter().enumerate() {
                if let Some(entry) = entry {
                    self.command_cache_expirations.push(Reverse((
                        entry.expires_at,
                        index,
                        entry.generation,
                    )));
                }
            }
        }
    }

    /// Resets the matcher buffer to a known boundary. Always use this
    /// instead of `self.buffer.clear()` directly: it also clears
    /// `buffer_truncated`, since a fresh boundary means whatever preceded it
    /// is no longer relevant to word-boundary checks.
    fn clear_buffer(&mut self) {
        self.zeroize_buffer_contents();
        self.buffer.clear();
        self.match_scratch.clear();
        self.buffered_bytes = 0;
        self.restored_reservation_chars = 0;
        self.non_ascii_buffer_chars = 0;
        self.buffer_truncated = false;
        self.clipboard_prefetch_armed = false;
    }

    fn zeroize_buffer_contents(&mut self) {
        for character in &mut self.buffer {
            character.zeroize();
        }
    }

    fn zeroize_undo_contents(&mut self) {
        if let Some((restore, erase)) = self.last_expansion.as_mut() {
            restore.zeroize();
            erase.zeroize();
        }
    }

    fn clear_undo_state(&mut self) {
        self.zeroize_undo_contents();
        self.last_expansion = None;
    }
}

impl Drop for ExpansionEngine {
    fn drop(&mut self) {
        self.clear_sensitive_runtime_state();
    }
}

impl ExpansionEngine {
    fn clear_sensitive_runtime_state(&mut self) {
        self.clear_buffer();
        self.clear_undo_state();
        for reservation in &mut self.deferred_matches {
            reservation.matched_text.zeroize();
        }
        self.deferred_matches.clear();
        // CommandCacheEntry wipes its output when each cache slot is dropped.
        self.command_cache.clear();
        self.command_cache_expirations.clear();
    }
}

fn pop_front_zeroized(buffer: &mut VecDeque<char>) -> Option<char> {
    let character = *buffer.front()?;
    buffer.front_mut()?.zeroize();
    buffer.pop_front();
    Some(character)
}

fn pop_back_zeroized(buffer: &mut VecDeque<char>) -> Option<char> {
    let character = *buffer.back()?;
    buffer.back_mut()?.zeroize();
    buffer.pop_back();
    Some(character)
}

#[cfg(test)]
mod tests;
