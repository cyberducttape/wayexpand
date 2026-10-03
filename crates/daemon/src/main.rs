mod backend_lifecycle;
mod control;
mod events;
mod input_loop;
mod latency;
mod output_loop;
mod policy;
mod reactor;
mod reload;
mod status;

use anyhow::Result;
use control::{ControlServer, FocusSnapshot};
use events::{
    apply_evdev_gating, apply_results, dispatch_pending_results, process_event,
    replay_evdev_follow_up, restore_abandoned_results, EvdevGatingOutcome,
};
use input_loop::{
    connect_evdev_with_retry, connect_input_method_session, connect_input_method_with_retry,
    input_poll_interval, next_retry_delay, spawn_stdin_reader, wait_for_retry,
};
use output_loop::{
    connect_output_backend, connect_output_with_retry, shutdown_injector, spawn_async_injector,
};
use reload::ReloadableConfig;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::{
    collections::hash_map::DefaultHasher,
    env,
    hash::{Hash, Hasher},
    io::{self, BufRead},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_evdev::EvdevSource;
use wayexpand_backend_libei::portal_token_path;
use wayexpand_backend_selection::auto_select;
use wayexpand_core::{
    default_config_path, CommandMetrics, ExpansionEngine, ExpansionResult, InjectorCapabilities,
    InputEvent, InputSource, InputSourceCapabilities, TextInjector, WindowContext,
};

/// How long to wait for physically held keys to be released before injecting
/// an expansion in evdev mode. Generous enough to cover a deliberate
/// keypress, bounded so a genuinely held key cannot stall expansion.
const KEY_RELEASE_TIMEOUT: Duration = Duration::from_millis(400);
/// Extra settling time for non-exclusive evdev capture. If another physical
/// event arrives during this window, the pending expansion is abandoned to
/// avoid deleting text from a cursor that has already moved.
const EVDEV_QUIET_TIMEOUT: Duration = Duration::from_millis(40);
const MAX_STDIN_LINE_BYTES: usize = 1024 * 1024;
const MAX_PENDING_INPUT_LINES: usize = 64;

fn commands_disabled_for_startup(
    policy: &wayexpand_core::OrganizationPolicy,
    worker_start_failed: bool,
) -> bool {
    worker_start_failed || policy::commands_enforced(policy)
}

#[derive(Debug)]
struct EventError {
    result: ExpansionResult,
    source: wayexpand_core::TransactionOutcome,
}

#[derive(Default)]
struct StatusPublisher {
    last: Option<StatusSnapshot>,
}

#[derive(Clone, PartialEq, Eq)]
struct StatusSnapshot {
    source: String,
    backend: String,
    backend_mode: String,
    state: String,
    paused: bool,
    config_path: PathBuf,
    config_healthy: bool,
    metrics: CommandMetrics,
    latency: latency::Snapshot,
    capture_capabilities: InputSourceCapabilities,
    injection_capabilities: InjectorCapabilities,
    window_tracker_connected: bool,
}

impl StatusPublisher {
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        control: &control::ControlServer,
        source: &str,
        backend: &str,
        backend_mode: &str,
        state: &str,
        config_path: &Path,
        config_healthy: bool,
        metrics: CommandMetrics,
        capture_capabilities: InputSourceCapabilities,
        injection_capabilities: InjectorCapabilities,
        window_tracker_connected: bool,
    ) {
        let snapshot = StatusSnapshot {
            source: source.to_owned(),
            backend: backend.to_owned(),
            backend_mode: backend_mode.to_owned(),
            state: state.to_owned(),
            paused: control
                .pause_requested
                .load(std::sync::atomic::Ordering::Acquire),
            config_path: config_path.to_path_buf(),
            config_healthy,
            metrics,
            latency: latency::snapshot(),
            capture_capabilities,
            injection_capabilities,
            window_tracker_connected,
        };
        if self.last.as_ref() == Some(&snapshot) {
            return;
        }
        control.set_status(status::daemon_status_body_with_runtime_capabilities(
            &snapshot.source,
            &snapshot.backend,
            &snapshot.state,
            snapshot.paused,
            snapshot.config_path.as_path(),
            snapshot.config_healthy,
            &snapshot.backend_mode,
            snapshot.metrics,
            snapshot.latency,
            snapshot.capture_capabilities,
            snapshot.injection_capabilities,
            snapshot.window_tracker_connected,
        ));
        self.last = Some(snapshot);
    }
}

impl std::fmt::Display for EventError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.source)
    }
}

impl std::error::Error for EventError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl EventError {
    /// Whether the output session failed in a way that a reconnect can fix.
    /// This is independent of whether the transaction may have partially
    /// applied: callers never replay the failed expansion either way, and a
    /// lost libei/wlroots connection (non-atomic backends) must reconnect
    /// rather than terminate the daemon.
    fn retryable(&self) -> bool {
        self.source.source().is_some_and(|source| source.retryable)
    }

    fn expansion_rejected(&self) -> bool {
        matches!(
            &self.source,
            wayexpand_core::TransactionOutcome::NotApplied { source }
                | wayexpand_core::TransactionOutcome::UnknownPartialFailure { source }
                if source.kind() == wayexpand_core::InjectorErrorKind::ExpansionRejected
        )
    }
}

fn main() -> Result<()> {
    // `fmt::init()` falls back to ERROR-only when RUST_LOG is unset, which
    // silently dropped every warning (policy violations, rejected reloads,
    // reconnects) from the journal. Default to info; RUST_LOG still wins.
    // Colour codes only belong on a terminal, not in journald.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();
    let (path, explicit_source, explicit_backend, allow_evdev_sensitive_fields, use_fleet) =
        parse_args()?;

    // Resolve automatic and partial explicit selections once. From this point
    // onward the daemon only consumes the canonical, compatible pair.
    let selection = auto_select(explicit_source.as_deref(), explicit_backend.as_deref())
        .map_err(|error| anyhow::anyhow!("backend selection failed: {error}"))?;
    info!("{}", selection.reason);
    let resolved_pair = selection.pair;
    let source_name = resolved_pair.source();
    let backend_name = resolved_pair.backend();

    if source_name == "evdev" && !allow_evdev_sensitive_fields {
        anyhow::bail!(
            "evdev cannot detect password or sensitive fields; refusing to start. ".to_owned()
                + "Use --allow-evdev-sensitive-fields only when your deployment accepts "
                + "that risk, or use --source=input-method for field-aware capture."
        );
    }
    if source_name == "evdev" {
        warn!(
            "evdev sensitive-field protection is unavailable; the explicit \
             --allow-evdev-sensitive-fields acknowledgement is active"
        );
    }

    // An absent policy is permissive; an existing invalid or insecure policy
    // is fatal so a management update cannot silently disable restrictions.
    let policy = policy::load_policy()
        .map_err(|error| anyhow::anyhow!("organization policy is invalid: {error}"))?;
    let policy_backend = wayexpand_core::policy_backend_name(source_name, backend_name);
    if !policy.backend_allowed(policy_backend) {
        let violation = format!(
            "backend '{policy_backend}' is not in allowed list: {:?}",
            policy.allowed_backends
        );
        policy::log_violation(&policy, &violation);
        if policy.safe_mode {
            // In safe_mode (enforcement), disallowed backends refuse startup
            anyhow::bail!("organization policy blocks startup: {}", violation);
        }
        // In audit mode, log the violation but continue
        info!("audit mode permits startup with disallowed backend");
    }

    let mut config = if use_fleet {
        ReloadableConfig::load_with_fleet_and_policy(&path, policy.clone())
    } else {
        ReloadableConfig::load_with_policy(&path, policy.clone())
    }
    .map_err(|_| {
        anyhow::anyhow!(
            "could not load configuration {}; run `wayexpand doctor` for details",
            path.display()
        )
    })?;
    let worker_start_failed = !config.engine.enable_async_commands();
    if worker_start_failed {
        warn!(
            "asynchronous command/hotkey workers could not start; command-backed actions are disabled"
        );
    }

    let enforcement_policy = policy.effective_enforcement_policy();
    config
        .engine
        .set_commands_disabled(commands_disabled_for_startup(&policy, worker_start_failed));
    config
        .engine
        .set_title_matching_disabled(enforcement_policy.disable_title_matching);

    // Reject impossible deployment guarantees before opening either the input
    // source or output backend. The input-method protocol is the only shipped
    // source that can provide both guarantees; the connected injector is
    // checked again below because output capabilities may be negotiated.
    let preflight_capabilities = if source_name == "input-method" {
        InjectorCapabilities {
            atomic_replace: true,
            full_unicode: true,
            ..InjectorCapabilities::default()
        }
    } else {
        InjectorCapabilities::default()
    };
    let preflight_source_capabilities = if source_name == "input-method" {
        InputSourceCapabilities::INPUT_METHOD_V2
    } else {
        InputSourceCapabilities::default()
    };
    if let Some(violation) = config
        .engine
        .capability_violation_for_source(preflight_capabilities, preflight_source_capabilities)
    {
        policy::log_violation(&policy, &violation);
        anyhow::bail!("organization policy blocks startup: {violation}");
    }

    let control = control::ControlServer::start()?;
    let managed = control.path().is_some();
    let signal_stop = control.stop_requested.clone();
    let mut signals = Signals::new([SIGINT, SIGTERM])
        .map_err(|error| anyhow::anyhow!("could not install signal handlers: {error}"))?;
    thread::spawn(move || {
        if signals.forever().next().is_some() {
            signal_stop.store(true, std::sync::atomic::Ordering::Release);
        }
    });
    info!(path = %config.path().display(), fleet = use_fleet, "configuration loaded");
    if let Some(socket) = control.path() {
        info!(path = %socket.display(), "control socket ready");
    } else {
        warn!("XDG_RUNTIME_DIR unavailable; control socket disabled");
    }
    let portal_token_path = portal_token_path();
    let mut input_method = match source_name {
        "input-method" => Some(connect_input_method_with_retry(
            &control,
            &path,
            config.healthy(),
            config.engine.libei_token_persistence(),
            portal_token_path.as_deref(),
            &policy,
        )?),
        _ => None,
    };
    let mut evdev = match source_name {
        "evdev" => Some(connect_evdev_with_retry(
            &control,
            &path,
            backend_name,
            config.healthy(),
        )?),
        _ => None,
    };
    match source_name {
        "stdin" | "input-method" | "evdev" => {}
        other => {
            anyhow::bail!("unknown source {other:?}; expected stdin, input-method, or evdev")
        }
    }
    let input_method_mode = source_name == "input-method";
    let evdev_mode = source_name == "evdev";
    if input_method_mode {
        info!(
            "input-method-v2 backend selected; libei key pass-through is mandatory for \
            unsupported keys (Escape, arrows, F-keys, shortcuts, etc.) to be re-injected"
        );
    }
    let active_source = source_name;
    let mut reconnect_delay = Duration::from_millis(250);
    let mut output_retry_at: Option<Instant> = None;
    let mut output_retry_delay = Duration::from_millis(250);
    let mut output_failures = None;
    let mut injector: Option<Box<dyn TextInjector>> = if input_method.is_some() {
        None
    } else {
        match backend_name {
            "none" => None,
            backend @ ("wlroots" | "libei") => {
                let Some(injector) = connect_output_with_retry(
                    &control,
                    active_source,
                    backend,
                    &path,
                    config.healthy(),
                    config.engine.libei_token_persistence(),
                    portal_token_path.as_deref(),
                )?
                else {
                    anyhow::bail!("output backend startup cancelled while stopping")
                };
                if evdev_mode && backend == "libei" {
                    let (injector, failures) = spawn_async_injector(injector);
                    output_failures = Some(failures);
                    Some(injector)
                } else {
                    Some(injector)
                }
            }
            other => {
                anyhow::bail!("unknown backend {other:?}; expected none, wlroots, or libei")
            }
        }
    };

    let injector_capabilities = input_method
        .as_ref()
        .map(TextInjector::capabilities)
        .or_else(|| injector.as_ref().map(|backend| backend.capabilities()))
        .unwrap_or_else(InjectorCapabilities::default);
    let source_capabilities = input_method
        .as_ref()
        .map(InputSource::capabilities)
        .or_else(|| evdev.as_ref().map(InputSource::capabilities))
        .unwrap_or_default();
    if let Some(violation) = config
        .engine
        .capability_violation_for_source(injector_capabilities, source_capabilities)
    {
        policy::log_violation(&policy, &violation);
        anyhow::bail!("organization policy blocks startup: {violation}");
    }

    let active_backend = input_method
        .as_ref()
        .map(|_| "input-method-v2")
        .or_else(|| injector.as_ref().map(|backend| backend.name()))
        .unwrap_or("none");
    let mut connection_state = if input_method_mode || evdev_mode {
        "connected"
    } else {
        "running"
    };
    let mut paused = false;
    let mut status_publisher = StatusPublisher::default();
    set_daemon_status(
        &mut status_publisher,
        &control,
        active_source,
        active_backend,
        connection_state,
        &path,
        config.healthy(),
    );
    info!(
        source = active_source,
        backend = active_backend,
        "input source active"
    );

    let receiver = if input_method.is_none() && evdev.is_none() {
        Some(spawn_stdin_reader())
    } else {
        None
    };

    let window_tracker = backend_lifecycle::spawn_window_tracker();
    let mut focus_state = FocusState {
        previous: config.engine.current_window().cloned(),
        generation: 0,
    };
    publish_focus_snapshot(&control, &focus_state);

    let mut stdin_closed = false;
    let mut logged_queue_rejections = 0;
    loop {
        drain_pending_window_events(
            &window_tracker,
            &mut config.engine,
            &policy,
            active_backend,
            &control,
            &mut focus_state,
        )?;
        let transition = reactor::ReactorTransition::sample(
            &control.stop_requested,
            &control.pause_requested,
            &control.reload_requested,
            paused,
        );
        if let Some(requested_pause) = transition.pause_changed() {
            process_event(
                &mut config.engine,
                InputEvent::PauseChanged(requested_pause),
                None,
                &policy,
                active_backend,
            )?;
            paused = requested_pause;
            info!(paused, "expansion processing policy changed");
        }
        if transition.reload_requested() {
            config.reload_now();
        }
        if transition.should_stop() {
            break;
        }
        config.reload_if_changed();
        for (action, result) in config.engine.drain_completed_hotkeys() {
            match result {
                Ok(()) => info!(chord = %action.chord, "hotkey action completed"),
                Err(error) => warn!(chord = %action.chord, %error, "hotkey action failed"),
            }
        }
        let metrics = config.engine.command_metrics();
        let worker_failure = output_failures
            .as_ref()
            .and_then(|failures| failures.try_recv().ok());
        if let Some(failure) = worker_failure {
            warn!(
                retryable = failure.retryable,
                error = %failure.message,
                "serialized output worker failed"
            );
            drop(injector.take());
            process_event(
                &mut config.engine,
                InputEvent::EndOfInput,
                None,
                &policy,
                active_backend,
            )?;
            output_failures = None;
            if !failure.retryable {
                return Err(anyhow::anyhow!(
                    "output backend failed permanently: {}",
                    failure.message
                ));
            }
            connection_state = "reconnecting";
            output_retry_at = Some(Instant::now());
            output_retry_delay = Duration::from_millis(250);
        }
        if metrics.command_queue_rejected_total < logged_queue_rejections {
            // A successful configuration reload creates a fresh engine and
            // therefore starts a fresh counter interval.
            logged_queue_rejections = 0;
        }
        if metrics.command_queue_rejected_total > logged_queue_rejections {
            warn!(
                command_queue_depth = metrics.command_queue_depth,
                command_queue_rejected_total = metrics.command_queue_rejected_total,
                "command action rejected because the command queue was full or unavailable"
            );
            logged_queue_rejections = metrics.command_queue_rejected_total;
        }
        let poll_interval = input_poll_interval(metrics);
        let completed_commands = config.engine.drain_completed_commands();
        if !completed_commands.is_empty() {
            // Apply evdev safety gating: ensure physical key-up was processed
            // and no competing input arrived during command execution.
            // This prevents the race condition where fast commands finish
            // before the trigger key's physical release event is processed.
            let gating = if evdev_mode {
                apply_evdev_gating(completed_commands, &mut evdev)
            } else {
                EvdevGatingOutcome {
                    results: completed_commands,
                    follow_up: Vec::new(),
                    abandoned: Vec::new(),
                }
            };
            restore_abandoned_results(&mut config.engine, gating.abandoned);

            if !gating.results.is_empty() {
                if input_method_mode {
                    if let Some(source) = input_method.as_mut() {
                        apply_results(
                            &mut config.engine,
                            gating.results,
                            Some(source),
                            &policy,
                            active_backend,
                        )?;
                    }
                } else if let Some(mut backend) = injector.take() {
                    let result = apply_results(
                        &mut config.engine,
                        gating.results,
                        Some(backend.as_mut()),
                        &policy,
                        active_backend,
                    );
                    injector = Some(backend);
                    result?;
                } else {
                    apply_results(
                        &mut config.engine,
                        gating.results,
                        None,
                        &policy,
                        active_backend,
                    )?;
                }
            }
            replay_evdev_follow_up(
                &mut config.engine,
                gating.follow_up,
                &policy,
                active_backend,
            )?;
        }
        if let Some(request) = control.take_insert_request() {
            // An explicit insert (quick-insert picker, `wayexpand insert`)
            // types a snippet at the cursor through the same injector and
            // evdev safety gate as a typed expansion. It is a user action,
            // so a refusal or injection failure is logged, never fatal.
            if let Some(expected_token) = request.focus_token.as_deref() {
                let snapshot = control.focus_snapshot();
                let current_token = config.engine.current_window().map(focus_token);
                if current_token.as_deref() != Some(expected_token)
                    || snapshot.token.as_deref() != Some(expected_token)
                    || snapshot.generation != request.focus_generation.unwrap_or_default()
                {
                    warn!(
                        expected = expected_token,
                        actual = ?current_token,
                        "requested snippet insert refused because focus changed"
                    );
                    continue;
                }
            }
            match config.engine.prepare_insert(&request.trigger) {
                Ok(result) => {
                    let gating = if evdev_mode {
                        apply_evdev_gating(vec![result], &mut evdev)
                    } else {
                        EvdevGatingOutcome {
                            results: vec![result],
                            follow_up: Vec::new(),
                            abandoned: Vec::new(),
                        }
                    };
                    if !gating.abandoned.is_empty() {
                        warn!("requested snippet insert abandoned because input arrived first");
                    }
                    let outcome = if gating.results.is_empty() {
                        Ok(())
                    } else if input_method_mode {
                        match input_method.as_mut() {
                            Some(source) => apply_results(
                                &mut config.engine,
                                gating.results,
                                Some(source),
                                &policy,
                                active_backend,
                            ),
                            None => {
                                warn!(
                                    "requested snippet insert skipped: input method reconnecting"
                                );
                                Ok(())
                            }
                        }
                    } else if let Some(mut backend) = injector.take() {
                        let outcome = apply_results(
                            &mut config.engine,
                            gating.results,
                            Some(backend.as_mut()),
                            &policy,
                            active_backend,
                        );
                        injector = Some(backend);
                        outcome
                    } else {
                        warn!("requested snippet insert skipped: no injection backend");
                        Ok(())
                    };
                    if let Err(error) = outcome {
                        warn!(%error, "requested snippet insert failed");
                    }
                    replay_evdev_follow_up(
                        &mut config.engine,
                        gating.follow_up,
                        &policy,
                        active_backend,
                    )?;
                }
                Err(error) => warn!(%error, "requested snippet insert refused"),
            }
        }
        set_daemon_status_with_runtime_capabilities(
            &mut status_publisher,
            &control,
            active_source,
            active_backend,
            connection_state,
            &path,
            config.healthy(),
            metrics,
            input_method
                .as_ref()
                .map(TextInjector::status_detail)
                .or_else(|| injector.as_ref().map(|backend| backend.status_detail()))
                .filter(|detail| !detail.is_empty())
                .unwrap_or("unknown"),
            input_method
                .as_ref()
                .map(InputSource::capabilities)
                .or_else(|| evdev.as_ref().map(InputSource::capabilities))
                .unwrap_or_default(),
            input_method
                .as_ref()
                .map(TextInjector::capabilities)
                .or_else(|| injector.as_ref().map(|backend| backend.capabilities()))
                .unwrap_or_default(),
            window_tracker
                .as_ref()
                .is_some_and(backend_lifecycle::WindowTrackerHandle::is_connected),
        );
        // Output recovery is deliberately one attempt per reactor turn. A
        // portal or compositor outage must not park control, reload, status,
        // or shutdown handling inside an exponential-backoff sleep.
        if injector.is_none()
            && !input_method_mode
            && backend_name != "none"
            && output_retry_at.is_some_and(|deadline| Instant::now() >= deadline)
        {
            match connect_output_backend(
                backend_name,
                config.engine.libei_token_persistence(),
                portal_token_path.as_deref(),
            ) {
                Ok(backend) => {
                    if evdev_mode && backend_name == "libei" {
                        let (backend, failures) = spawn_async_injector(backend);
                        injector = Some(backend);
                        output_failures = Some(failures);
                    } else {
                        injector = Some(backend);
                    }
                    output_retry_at = None;
                    output_retry_delay = Duration::from_millis(250);
                    connection_state = "connected";
                    set_daemon_status(
                        &mut status_publisher,
                        &control,
                        active_source,
                        backend_name,
                        connection_state,
                        &path,
                        config.healthy(),
                    );
                    info!(backend = backend_name, "output backend reconnected");
                }
                Err(error) if error.retryable => {
                    output_retry_at = Some(Instant::now() + output_retry_delay);
                    output_retry_delay = next_retry_delay(output_retry_delay);
                    warn!(%error, backend = backend_name, "output backend unavailable; retry scheduled");
                }
                Err(error) => return Err(anyhow::Error::new(error)),
            }
        }
        if input_method_mode {
            if input_method.is_none() {
                match connect_input_method_session(
                    &control,
                    &path,
                    config.healthy(),
                    config.engine.libei_token_persistence(),
                    portal_token_path.as_deref(),
                    &policy,
                ) {
                    Ok(source) => {
                        input_method = Some(source);
                        reconnect_delay = Duration::from_millis(250);
                        connection_state = "connected";
                        set_daemon_status(
                            &mut status_publisher,
                            &control,
                            active_source,
                            active_backend,
                            connection_state,
                            &path,
                            config.healthy(),
                        );
                        info!("input-method source reconnected");
                    }
                    Err(error) if error.is_retryable() => {
                        warn!(%error, "input-method unavailable; retrying");
                        if !wait_for_retry(&control.stop_requested, reconnect_delay) {
                            break;
                        }
                        reconnect_delay = next_retry_delay(reconnect_delay);
                    }
                    Err(error) => {
                        return Err(anyhow::anyhow!(
                            "input-method reconnect failed permanently: {error}"
                        ));
                    }
                }
                continue;
            }
            let Some(source) = input_method.as_mut() else {
                return Err(anyhow::anyhow!("input-method mode lost its input source"));
            };
            let event_result = source.next_event_timeout(poll_interval);
            match event_result {
                Ok(Some(event)) => {
                    drain_pending_window_events(
                        &window_tracker,
                        &mut config.engine,
                        &policy,
                        active_backend,
                        &control,
                        &mut focus_state,
                    )?;
                    let result = match input_method.as_mut() {
                        Some(source) => process_event(
                            &mut config.engine,
                            event,
                            Some(source),
                            &policy,
                            active_backend,
                        ),
                        None => {
                            return Err(anyhow::anyhow!(
                                "input-method source disappeared while processing an event"
                            ));
                        }
                    };
                    match result {
                        Ok(()) => reconnect_delay = Duration::from_millis(250),
                        Err(error) if error.expansion_rejected() => {
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "input-method rejected expansion; continuing"
                            );
                            reconnect_delay = Duration::from_millis(250);
                        }
                        Err(error) if error.retryable() => {
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "input-method output failed; current expansion is not replayed"
                            );
                            input_method = None;
                            connection_state = "reconnecting";
                            process_event(
                                &mut config.engine,
                                InputEvent::FocusChanged { sensitive: true },
                                None,
                                &policy,
                                active_backend,
                            )?;
                            set_daemon_status(
                                &mut status_publisher,
                                &control,
                                active_source,
                                active_backend,
                                connection_state,
                                &path,
                                config.healthy(),
                            );
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Ok(None) => {}
                Err(error) if error.retryable => {
                    warn!(%error, "input-method connection lost; reconnecting");
                    input_method = None;
                    connection_state = "reconnecting";
                    process_event(
                        &mut config.engine,
                        InputEvent::FocusChanged { sensitive: true },
                        None,
                        &policy,
                        active_backend,
                    )?;
                    set_daemon_status(
                        &mut status_publisher,
                        &control,
                        active_source,
                        active_backend,
                        connection_state,
                        &path,
                        config.healthy(),
                    );
                }
                Err(error) => {
                    return Err(anyhow::anyhow!("input source failed: {error}"));
                }
            }
            continue;
        }
        if evdev_mode {
            if evdev.is_none() {
                match EvdevSource::connect() {
                    Ok(source) => {
                        evdev = Some(source);
                        reconnect_delay = Duration::from_millis(250);
                        connection_state = "connected";
                        set_daemon_status(
                            &mut status_publisher,
                            &control,
                            active_source,
                            active_backend,
                            connection_state,
                            &path,
                            config.healthy(),
                        );
                        info!("evdev source reconnected");
                    }
                    Err(error) if error.is_retryable() => {
                        warn!(%error, "evdev source unavailable; retrying");
                        if !wait_for_retry(&control.stop_requested, reconnect_delay) {
                            break;
                        }
                        reconnect_delay = next_retry_delay(reconnect_delay);
                    }
                    Err(error) => {
                        return Err(anyhow::anyhow!(
                            "evdev reconnect failed permanently: {error}"
                        ));
                    }
                }
                continue;
            }
            let Some(source) = evdev.as_mut() else {
                return Err(anyhow::anyhow!("evdev mode lost its input source"));
            };
            let event_result = source.next_event_timeout(poll_interval);
            match event_result {
                Ok(Some(event)) => {
                    drain_pending_window_events(
                        &window_tracker,
                        &mut config.engine,
                        &policy,
                        active_backend,
                        &control,
                        &mut focus_state,
                    )?;
                    let result = if let Some(mut backend) = injector.take() {
                        // Match immediately. The release/quiet gates are only
                        // needed if the matcher actually produced text that
                        // will modify the focused application.
                        let result = if matches!(event, InputEvent::Key(_)) {
                            process_event(
                                &mut config.engine,
                                event,
                                Some(backend.as_mut()),
                                &policy,
                                active_backend,
                            )
                        } else {
                            let pending = config.engine.process_deferred(event);
                            let results = dispatch_pending_results(
                                &mut config.engine,
                                pending,
                                &policy,
                                active_backend,
                            );
                            if results.is_empty() {
                                Ok(())
                            } else {
                                let gating = apply_evdev_gating(results, &mut evdev);
                                restore_abandoned_results(&mut config.engine, gating.abandoned);
                                apply_results(
                                    &mut config.engine,
                                    gating.results,
                                    Some(backend.as_mut()),
                                    &policy,
                                    active_backend,
                                )?;
                                replay_evdev_follow_up(
                                    &mut config.engine,
                                    gating.follow_up,
                                    &policy,
                                    active_backend,
                                )
                            }
                        };
                        injector = Some(backend);
                        result
                    } else {
                        process_event(&mut config.engine, event, None, &policy, active_backend)
                    };
                    match result {
                        Ok(()) => reconnect_delay = Duration::from_millis(250),
                        Err(error) if error.expansion_rejected() => {
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "evdev rejected expansion; continuing"
                            );
                            reconnect_delay = Duration::from_millis(250);
                        }
                        Err(error) if error.retryable() => {
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "evdev output failed; current expansion is not replayed"
                            );
                            drop(injector.take());
                            process_event(
                                &mut config.engine,
                                InputEvent::EndOfInput,
                                None,
                                &policy,
                                active_backend,
                            )?;
                            connection_state = "reconnecting";
                            set_daemon_status(
                                &mut status_publisher,
                                &control,
                                active_source,
                                active_backend,
                                connection_state,
                                &path,
                                config.healthy(),
                            );
                            output_retry_at = Some(Instant::now());
                            output_retry_delay = Duration::from_millis(250);
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Ok(None) => {}
                Err(error) if error.retryable => {
                    warn!(%error, "evdev connection lost; reconnecting");
                    evdev = None;
                    connection_state = "reconnecting";
                    let _ = process_event(
                        &mut config.engine,
                        InputEvent::EndOfInput,
                        None,
                        &policy,
                        active_backend,
                    );
                    set_daemon_status(
                        &mut status_publisher,
                        &control,
                        active_source,
                        active_backend,
                        connection_state,
                        &path,
                        config.healthy(),
                    );
                }
                Err(error) => {
                    return Err(anyhow::anyhow!("input source failed: {error}"));
                }
            }
            continue;
        }
        if stdin_closed {
            thread::sleep(poll_interval);
            continue;
        }
        let Some(receiver) = receiver.as_ref() else {
            break;
        };
        match receiver.recv_timeout(poll_interval) {
            Ok(line) => {
                drain_pending_window_events(
                    &window_tracker,
                    &mut config.engine,
                    &policy,
                    active_backend,
                    &control,
                    &mut focus_state,
                )?;
                if injector.is_some() {
                    for character in line.chars() {
                        let event = InputEvent::Text(character.to_string());
                        let (result, backend) = if let Some(mut backend) = injector.take() {
                            let result = process_event(
                                &mut config.engine,
                                event,
                                Some(backend.as_mut()),
                                &policy,
                                active_backend,
                            );
                            (result, Some(backend))
                        } else {
                            (
                                process_event(
                                    &mut config.engine,
                                    event,
                                    None,
                                    &policy,
                                    active_backend,
                                ),
                                None,
                            )
                        };
                        injector = backend;
                        if let Err(error) = result {
                            if error.expansion_rejected() {
                                warn!(
                                    error = %error,
                                    trigger_chars = error.result.trigger.chars().count(),
                                    insert_bytes = error.result.insert.len(),
                                    "output backend rejected expansion; continuing"
                                );
                                continue;
                            }
                            if !error.retryable() {
                                return Err(error.into());
                            }
                            warn!(
                                error = %error,
                                trigger_chars = error.result.trigger.chars().count(),
                                insert_bytes = error.result.insert.len(),
                                "output session failed; current expansion is not replayed"
                            );
                            drop(injector.take());
                            let _ = process_event(
                                &mut config.engine,
                                InputEvent::EndOfInput,
                                None,
                                &policy,
                                active_backend,
                            );
                            connection_state = "reconnecting";
                            set_daemon_status(
                                &mut status_publisher,
                                &control,
                                active_source,
                                backend_name,
                                connection_state,
                                &path,
                                config.healthy(),
                            );
                            output_retry_at = Some(Instant::now());
                            output_retry_delay = Duration::from_millis(250);
                        }
                    }
                    if let Some(backend) = injector.as_deref_mut() {
                        process_event(
                            &mut config.engine,
                            InputEvent::EndOfInput,
                            Some(backend),
                            &policy,
                            active_backend,
                        )?;
                    }
                } else {
                    process_event(
                        &mut config.engine,
                        InputEvent::Text(line),
                        None,
                        &policy,
                        active_backend,
                    )?;
                    process_event(
                        &mut config.engine,
                        InputEvent::EndOfInput,
                        None,
                        &policy,
                        active_backend,
                    )?;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) if managed => {
                warn!("stdin input source ended; daemon remains idle under control socket");
                stdin_closed = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    warn!("input stream ended; daemon stopping");
    // Backends get an explicit teardown opportunity. The bounded wait keeps
    // systemd stop independent from a broken portal implementation, while
    // still allowing libei to close its portal session and Tokio runtime
    // cleanly in the normal case.
    if let Some(injector) = injector.take() {
        shutdown_injector(injector);
    }
    Ok(())
}

/// The last published focused window and a counter bumped on every change,
/// so the quick-insert picker can tell focus left and came back.
struct FocusState {
    previous: Option<WindowContext>,
    generation: u64,
}

fn focus_token(window: &WindowContext) -> String {
    let mut hasher = DefaultHasher::new();
    window.app_id.hash(&mut hasher);
    window.title.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn publish_focus_snapshot(control: &ControlServer, state: &FocusState) {
    control.set_focus_snapshot(FocusSnapshot {
        generation: state.generation,
        token: state.previous.as_ref().map(focus_token),
    });
}

/// Drain any pending window-change events from the tracker's receiver
/// and apply them to the engine. This prevents app-filter races where a
/// focus change arrives between input-event wait and processing.
fn drain_pending_window_events(
    window_tracker: &Option<backend_lifecycle::WindowTrackerHandle>,
    engine: &mut ExpansionEngine,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
    control: &ControlServer,
    focus_state: &mut FocusState,
) -> Result<()> {
    let receiver = window_tracker.as_ref().map(|tracker| &tracker.receiver);
    if let Some(window_opt) = backend_lifecycle::drain_pending_window_events(receiver) {
        process_event(
            engine,
            InputEvent::WindowChanged(window_opt),
            None,
            policy,
            active_backend,
        )?;
    }
    // Runs on every loop iteration: compare by reference and publish only on
    // an actual focus change.
    if focus_state.previous.as_ref() != engine.current_window() {
        focus_state.previous = engine.current_window().cloned();
        focus_state.generation = focus_state.generation.wrapping_add(1);
        publish_focus_snapshot(control, focus_state);
    }
    Ok(())
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut bytes = Vec::with_capacity(4096);
    let mut oversized = false;

    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(chunk.len(), |index| index + 1);
        if !oversized {
            let remaining = MAX_STDIN_LINE_BYTES + 1 - bytes.len();
            let copied = consumed.min(remaining);
            bytes.extend_from_slice(&chunk[..copied]);
            if bytes.len() > MAX_STDIN_LINE_BYTES {
                oversized = true;
                bytes.clear();
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if oversized {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("stdin line exceeds {MAX_STDIN_LINE_BYTES} bytes"),
        ));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn set_daemon_status(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
) {
    set_daemon_status_with_metrics(
        publisher,
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        CommandMetrics::default(),
    );
}

#[allow(clippy::too_many_arguments)]
fn set_daemon_status_with_metrics(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
) {
    let connected = state == "connected";
    let backend_mode = if connected {
        publisher.last.as_ref().map_or_else(
            || "unknown".to_owned(),
            |snapshot| snapshot.backend_mode.clone(),
        )
    } else {
        "unknown".to_owned()
    };
    let capture_capabilities = if connected {
        publisher
            .last
            .as_ref()
            .map_or_else(InputSourceCapabilities::default, |snapshot| {
                snapshot.capture_capabilities
            })
    } else {
        InputSourceCapabilities::default()
    };
    let injection_capabilities = if connected {
        publisher
            .last
            .as_ref()
            .map_or_else(InjectorCapabilities::default, |snapshot| {
                snapshot.injection_capabilities
            })
    } else {
        InjectorCapabilities::default()
    };
    let window_tracker_connected = publisher
        .last
        .as_ref()
        .is_some_and(|snapshot| snapshot.window_tracker_connected);
    publisher.publish(
        control,
        source,
        backend,
        &backend_mode,
        state,
        config_path,
        config_healthy,
        metrics,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

#[allow(clippy::too_many_arguments)]
fn set_daemon_status_with_runtime_capabilities(
    publisher: &mut StatusPublisher,
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
    metrics: CommandMetrics,
    backend_mode: &str,
    capture_capabilities: InputSourceCapabilities,
    injection_capabilities: InjectorCapabilities,
    window_tracker_connected: bool,
) {
    publisher.publish(
        control,
        source,
        backend,
        backend_mode,
        state,
        config_path,
        config_healthy,
        metrics,
        capture_capabilities,
        injection_capabilities,
        window_tracker_connected,
    );
}

type DaemonArgs = (PathBuf, Option<String>, Option<String>, bool, bool);

fn parse_args() -> Result<DaemonArgs> {
    let env_path = env::var_os("WAYEXPAND_CONFIG").map(PathBuf::from);
    let mut path = env_path.clone();
    let mut explicit_path = env_path.is_some();
    let mut backend = env::var("WAYEXPAND_BACKEND").ok();
    let mut source = env::var("WAYEXPAND_SOURCE").ok();
    let mut allow_evdev_sensitive_fields = false;
    for argument in env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--backend=") {
            backend = Some(value.to_string());
        } else if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_string());
        } else if argument == "--allow-evdev-sensitive-fields" {
            allow_evdev_sensitive_fields = true;
        } else if matches!(argument.as_str(), "--help" | "-h") {
            println!("wayexpand-daemon {}\nusage: wayexpand-daemon [--source=stdin|input-method|evdev] [--backend=none|wlroots|libei] [--allow-evdev-sensitive-fields] [config]", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        } else if matches!(argument.as_str(), "--version" | "-V") {
            println!("wayexpand-daemon {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        } else if argument.starts_with('-') {
            anyhow::bail!("unknown option {argument:?}; try --help");
        } else if path.is_some() {
            anyhow::bail!("multiple configuration paths supplied");
        } else {
            path = Some(PathBuf::from(argument));
            explicit_path = true;
        }
    }
    Ok((
        path.unwrap_or_else(default_config_path),
        source,
        backend,
        allow_evdev_sensitive_fields,
        !explicit_path,
    ))
}

#[cfg(test)]
mod tests;
