mod control;
mod policy;
mod reload;
mod status;

use anyhow::Result;
use reload::ReloadableConfig;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::{
    env,
    io::{self, BufRead},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_evdev::EvdevSource;
use wayexpand_backend_input_method::{InputMethodError, InputMethodSource};
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_backend_libei::{portal_token_path, LibeiInjector, LibeiOptions};
use wayexpand_backend_selection::auto_select;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::{
    default_config_path, CommandMetrics, ExpansionEngine, ExpansionError, ExpansionResult,
    InputEvent, TextInjector, WindowContext, WindowTracker,
};

/// How long to wait for physically held keys to be released before injecting
/// an expansion in evdev mode. Generous enough to cover a deliberate
/// keypress, bounded so a genuinely held key cannot stall expansion.
const KEY_RELEASE_TIMEOUT: Duration = Duration::from_millis(400);
/// Low-latency polling while command/hotkey work is queued or running.
/// Long-term, command completion should wake the input loop directly; until
/// then, keep the fast cadence only while there is async work to collect.
const ACTIVE_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Idle maintenance cadence for reload/pause/stop checks and completed command
/// collection when no command or hotkey job is known to be pending.
const IDLE_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);
/// Extra settling time for non-exclusive evdev capture. If another physical
/// event arrives during this window, the pending expansion is abandoned to
/// avoid deleting text from a cursor that has already moved.
const EVDEV_QUIET_TIMEOUT: Duration = Duration::from_millis(40);
const MAX_STDIN_LINE_BYTES: usize = 1024 * 1024;
const MAX_PENDING_INPUT_LINES: usize = 64;

fn evdev_release_is_safe<E: std::fmt::Display>(release: Result<(), E>) -> bool {
    match release {
        Ok(()) => true,
        Err(error) => {
            warn!(
                %error,
                "waiting for key release failed; dropping expansion because safe injection cannot be verified"
            );
            false
        }
    }
}

fn commands_disabled_for_startup(
    policy: &wayexpand_core::OrganizationPolicy,
    worker_start_failed: bool,
) -> bool {
    worker_start_failed || policy::commands_enforced(policy)
}

#[derive(Debug)]
struct EventError {
    result: ExpansionResult,
    source: ExpansionError,
}

#[derive(Debug)]
struct OutputConnectError {
    message: String,
    retryable: bool,
}

#[derive(Default)]
struct StatusPublisher {
    last: Option<StatusSnapshot>,
}

#[derive(Clone, PartialEq, Eq)]
struct StatusSnapshot {
    source: String,
    backend: String,
    state: String,
    paused: bool,
    config_path: PathBuf,
    config_healthy: bool,
    metrics: CommandMetrics,
}

impl StatusPublisher {
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        control: &control::ControlServer,
        source: &str,
        backend: &str,
        state: &str,
        config_path: &Path,
        config_healthy: bool,
        metrics: CommandMetrics,
    ) {
        let snapshot = StatusSnapshot {
            source: source.to_owned(),
            backend: backend.to_owned(),
            state: state.to_owned(),
            paused: control
                .pause_requested
                .load(std::sync::atomic::Ordering::Acquire),
            config_path: config_path.to_path_buf(),
            config_healthy,
            metrics,
        };
        if self.last.as_ref() == Some(&snapshot) {
            return;
        }
        status::set_daemon_status(
            control,
            &snapshot.source,
            &snapshot.backend,
            &snapshot.state,
            snapshot.config_path.as_path(),
            snapshot.config_healthy,
            snapshot.metrics,
        );
        self.last = Some(snapshot);
    }
}

impl std::fmt::Display for OutputConnectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OutputConnectError {}

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
    fn retryable(&self) -> bool {
        match &self.source {
            ExpansionError::Injection(error) => error.retryable,
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let (path, explicit_source, explicit_backend, use_fleet) = parse_args()?;

    // Resolve automatic and partial explicit selections once. From this point
    // onward the daemon only consumes the canonical, compatible pair.
    let selection = auto_select(explicit_source.as_deref(), explicit_backend.as_deref())
        .map_err(|error| anyhow::anyhow!("backend selection failed: {error}"))?;
    info!("{}", selection.reason);
    let resolved_pair = selection.pair;
    let source_name = resolved_pair.source();
    let backend_name = resolved_pair.backend();

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
    // evdev observes keystrokes non-exclusively. The focused application will
    // receive the terminating punctuation itself, so do not erase and
    // synthesize that character as part of the replacement.
    config
        .engine
        .set_reinsert_terminators(source_name != "evdev");
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
    if input_method_mode {
        info!(
            "input-method-v2 backend selected; optional libei key pass-through allows \
            unsupported keys (Escape, arrows, F-keys, shortcuts, etc.) to be re-injected"
        );
    }
    let evdev_mode = source_name == "evdev";
    let active_source = source_name;
    let mut reconnect_delay = Duration::from_millis(250);
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
                Some(injector)
            }
            other => {
                anyhow::bail!("unknown backend {other:?}; expected none, wlroots, or libei")
            }
        }
    };

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
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_INPUT_LINES);
        thread::spawn(move || {
            let mut reader = io::BufReader::new(io::stdin().lock());
            loop {
                match read_bounded_line(&mut reader) {
                    Ok(Some(line)) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        warn!(%error, "stdin line rejected");
                    }
                }
            }
        });
        Some(receiver)
    } else {
        None
    };

    let window_tracker = spawn_window_tracker();

    let mut stdin_closed = false;
    let mut logged_queue_rejections = 0;
    loop {
        if let Some(receiver) = window_tracker.as_ref() {
            let mut latest = None;
            while let Ok(window) = receiver.try_recv() {
                latest = Some(window);
            }
            if let Some(window) = latest {
                process_event(
                    &mut config.engine,
                    InputEvent::WindowChanged(window),
                    None,
                    &policy,
                    active_backend,
                )?;
            }
        }
        let requested_pause = control
            .pause_requested
            .load(std::sync::atomic::Ordering::Acquire);
        if requested_pause != paused {
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
        if control
            .reload_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            config.reload_now();
        }
        if control
            .stop_requested
            .load(std::sync::atomic::Ordering::Acquire)
        {
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
        set_daemon_status_with_metrics(
            &mut status_publisher,
            &control,
            active_source,
            active_backend,
            connection_state,
            &path,
            config.healthy(),
            metrics,
        );
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
                            let Some(reconnected) = connect_output_with_retry(
                                &control,
                                active_source,
                                backend_name,
                                &path,
                                config.healthy(),
                                config.engine.libei_token_persistence(),
                                portal_token_path.as_deref(),
                            )?
                            else {
                                break;
                            };
                            injector = Some(reconnected);
                            connection_state = "connected";
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
                            let Some(reconnected) = connect_output_with_retry(
                                &control,
                                active_source,
                                backend_name,
                                &path,
                                config.healthy(),
                                config.engine.libei_token_persistence(),
                                portal_token_path.as_deref(),
                            )?
                            else {
                                break;
                            };
                            injector = Some(reconnected);
                            connection_state = "connected";
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
    // The libei backend's Drop can hang indefinitely when connected through
    // a desktop portal (e.g. KWin's RemoteDesktop portal): its
    // `tokio::runtime::Runtime` blocks the dropping thread until its
    // background tasks reach a safe stopping point, which observably does
    // not always happen promptly against every portal implementation. Left
    // inline, that stalls this function's return past systemd's
    // `TimeoutStopSec`, forcing a SIGKILL instead of the clean exit this
    // service is asking for. Move the injector's drop to a detached thread
    // so a hang there can never delay `control`'s own drop just below
    // (which removes the control socket file -- needed for a clean
    // restart) or the daemon's own exit; the whole process going away
    // reclaims that thread regardless of whether its drop ever finishes.
    if let Some(injector) = injector.take() {
        thread::spawn(move || drop(injector));
    }
    Ok(())
}

/// Starts the focused-window tracker in the background when one is
/// available, feeding `WindowChanged` events into the main loop through a
/// channel so `app_filter`-scoped expansions can gate on it. Returns `None`
/// (not an error) when no tracker applies to this session -- window
/// tracking is inherently compositor-specific. Today, only KDE Plasma (KWin)
/// is integrated; wlroots tracking remains a future experimental project.
/// `app_filter`-scoped expansions simply fail closed everywhere else, exactly
/// as they would if this thread were never started.
fn spawn_window_tracker() -> Option<mpsc::Receiver<Option<WindowContext>>> {
    // Try the integrated KDE Plasma backend. The wlroots toplevel prototype is
    // deliberately not part of the production daemon until its event-loop,
    // ownership, and compositor test coverage are complete.

    // Try KWin first
    if KwinWindowTracker::probe().is_ok() {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut tracker = match KwinWindowTracker::new() {
                Ok(tracker) => tracker,
                Err(error) => {
                    warn!(%error, "KWin window tracker failed to start after a successful probe");
                    return;
                }
            };
            info!("window tracker active (KWin scripting bridge)");
            loop {
                match tracker.next_window_timeout(Duration::from_secs(2)) {
                    Ok(Some(window)) => {
                        if sender.send(window).is_err() {
                            break;
                        }
                    }
                    // Nothing changed within the timeout: expected and frequent.
                    Ok(None) => {}
                    Err(error) => {
                        warn!(%error, "KWin window tracker stopped");
                        break;
                    }
                }
            }
        });
        return Some(receiver);
    }

    info!("window tracking unavailable; app_filter-scoped expansions will not match");
    None
}

/// Drain any pending window-change events from the tracker's receiver
/// and apply them to the engine. This prevents app-filter races where a
/// focus change arrives between input-event wait and processing.
fn drain_pending_window_events(
    window_tracker: &Option<mpsc::Receiver<Option<WindowContext>>>,
    engine: &mut ExpansionEngine,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> Result<()> {
    if let Some(receiver) = window_tracker.as_ref() {
        let mut latest = None;
        loop {
            match receiver.try_recv() {
                Ok(window) => latest = Some(window),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    latest = Some(None);
                    break;
                }
            }
        }
        if let Some(window) = latest {
            process_event(
                engine,
                InputEvent::WindowChanged(window),
                None,
                policy,
                active_backend,
            )?;
        }
    }
    Ok(())
}

fn next_retry_delay(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(Duration::from_secs(30))
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
    publisher.publish(
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        metrics,
    );
}

fn set_daemon_status_direct(
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
) {
    status::set_daemon_status(
        control,
        source,
        backend,
        state,
        config_path,
        config_healthy,
        CommandMetrics::default(),
    );
}

fn input_poll_interval(metrics: CommandMetrics) -> Duration {
    if metrics.command_queue_depth > 0 || metrics.command_in_flight > 0 {
        ACTIVE_COMPLETION_POLL_INTERVAL
    } else {
        IDLE_MAINTENANCE_INTERVAL
    }
}

fn connect_input_method_session(
    control: &control::ControlServer,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
    policy: &wayexpand_core::OrganizationPolicy,
) -> Result<InputMethodSource, InputMethodError> {
    let mut source = InputMethodSource::connect()?;

    if !policy.backend_allowed("libei") {
        let violation = format!(
            "backend 'libei' is not in allowed list: {:?}",
            policy.allowed_backends
        );
        policy::log_violation(policy, &violation);
        if libei_policy_blocks(policy) {
            warn!(
                "organization policy prohibits libei backend; \
                unsupported keys will not pass through"
            );
            return Ok(source);
        }
        info!("audit mode permits libei key pass-through with a disallowed backend");
    }

    match connect_output_backend("libei", persist_portal_token, portal_token_path) {
        Ok(key_injector) => {
            set_daemon_status_direct(
                control,
                "input-method",
                "libei",
                "connected",
                config_path,
                config_healthy,
            );
            source = source.with_key_pass_through(key_injector);
        }
        Err(error) if error.retryable => {
            warn!(
                %error,
                "libei unavailable at startup; unsupported keys will not pass through \
                (connection will be retried asynchronously)"
            );
            set_daemon_status_direct(
                control,
                "input-method",
                "libei",
                "degraded",
                config_path,
                config_healthy,
            );
        }
        Err(error) => {
            return Err(InputMethodError::Protocol(format!(
                "libei unavailable: {}",
                error.message
            )));
        }
    }

    Ok(source)
}

fn libei_policy_blocks(policy: &wayexpand_core::OrganizationPolicy) -> bool {
    policy.safe_mode && !policy.backend_allowed("libei")
}

fn connect_input_method_with_retry(
    control: &control::ControlServer,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
    policy: &wayexpand_core::OrganizationPolicy,
) -> Result<InputMethodSource> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match connect_input_method_session(
            control,
            config_path,
            config_healthy,
            persist_portal_token,
            portal_token_path,
            policy,
        ) {
            Ok(source) => {
                control.set_status(format!(
                    "source=input-method\nbackend=input-method-v2\nstate=connected\npaused=false\nconfig={}\nconfig_state=ok",
                    config_path.display()
                ));
                return Ok(source);
            }
            Err(error) if error.is_retryable() => {
                warn!(%error, ?retry_delay, "input-method unavailable at startup; retrying");
                control.set_status(format!(
                    "source=input-method\nbackend=input-method-v2\nstate=reconnecting\npaused=false\nconfig={}\nconfig_state=ok",
                    config_path.display()
                ));
                if !wait_for_retry(&control.stop_requested, retry_delay) {
                    anyhow::bail!("input-method startup cancelled while waiting to reconnect");
                }
                retry_delay = next_retry_delay(retry_delay);
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "connecting input-method-v2 source failed permanently: {error}"
                ));
            }
        }
    }
}

fn connect_evdev_with_retry(
    control: &control::ControlServer,
    config_path: &Path,
    backend_name: &str,
    config_healthy: bool,
) -> Result<EvdevSource> {
    let backend = backend_name;
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match EvdevSource::connect() {
            Ok(source) => {
                set_daemon_status_direct(
                    control,
                    "evdev",
                    backend,
                    "connected",
                    config_path,
                    config_healthy,
                );
                return Ok(source);
            }
            Err(error) if error.is_retryable() => {
                warn!(%error, ?retry_delay, "evdev source unavailable at startup; retrying");
                set_daemon_status_direct(
                    control,
                    "evdev",
                    backend,
                    "reconnecting",
                    config_path,
                    config_healthy,
                );
                if !wait_for_retry(&control.stop_requested, retry_delay) {
                    anyhow::bail!("evdev startup cancelled while waiting to reconnect");
                }
                retry_delay = next_retry_delay(retry_delay);
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "connecting evdev source failed permanently: {error}"
                ));
            }
        }
    }
}

fn wait_for_retry(stop: &std::sync::atomic::AtomicBool, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        thread::sleep(remaining.min(Duration::from_millis(250)));
    }
    false
}

fn connect_output_backend(
    name: &str,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
) -> std::result::Result<Box<dyn TextInjector>, OutputConnectError> {
    match name {
        "wlroots" => WlrootsInjector::connect()
            .map(|injector| Box::new(injector) as Box<dyn TextInjector>)
            .map_err(|error| OutputConnectError {
                retryable: error.is_retryable(),
                message: format!("connecting wlroots output backend: {error}"),
            }),
        "libei" => LibeiInjector::connect(LibeiOptions {
            persist_portal_token,
            portal_token_path: portal_token_path.map(Path::to_path_buf),
        })
        .map(|injector| Box::new(injector) as Box<dyn TextInjector>)
        .map_err(|error| OutputConnectError {
            retryable: error.is_retryable(),
            message: format!("connecting libei output backend: {error}"),
        }),
        other => Err(OutputConnectError {
            retryable: false,
            message: format!("unknown output backend {other:?}"),
        }),
    }
}

fn connect_output_with_retry(
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
) -> Result<Option<Box<dyn TextInjector>>> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match connect_output_backend(backend, persist_portal_token, portal_token_path) {
            Ok(injector) => {
                set_daemon_status_direct(
                    control,
                    source,
                    backend,
                    "connected",
                    config_path,
                    config_healthy,
                );
                info!(backend, "output backend reconnected");
                return Ok(Some(injector));
            }
            Err(error) if error.retryable => {
                warn!(%error, backend, ?retry_delay, "output backend unavailable; retrying");
                set_daemon_status_direct(
                    control,
                    source,
                    backend,
                    "reconnecting",
                    config_path,
                    config_healthy,
                );
                if !wait_for_retry(&control.stop_requested, retry_delay) {
                    return Ok(None);
                }
                retry_delay = next_retry_delay(retry_delay);
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

fn text_contains_newlines(text: &str) -> bool {
    text.contains('\n') || text.contains('\r')
}

fn process_event(
    engine: &mut ExpansionEngine,
    event: InputEvent,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    if let InputEvent::Key(chord) = event {
        for action in engine.process_key(&chord) {
            // Apply the organization policy only to configured hotkey
            // actions. Ordinary evdev key events must not be treated as
            // hotkey violations, and undo is handled independently below.
            if let Err(violation) = policy::check_hotkey_allowed(policy) {
                policy::log_violation(policy, &violation);
                if policy.safe_mode {
                    continue;
                }
            }
            if let Some(violation) = policy.command_path_violation(&action.command.program) {
                policy::log_violation(policy, &violation);
                if policy.command_path_is_blocked(&action.command.program) {
                    continue;
                }
            }
            if let Err(error) = engine.queue_hotkey(&action) {
                warn!(
                    chord = %action.chord,
                    %error,
                    "hotkey action was not queued"
                );
            }
        }
        // Undo is a transaction too: do not consume the undo record until
        // there is an injector and the replacement has been applied. This
        // preserves retryability across backend reconnects and failures.
        if injector.is_some() {
            if let Some(result) = engine.prepare_undo(&chord) {
                if let Some(backend) = injector.as_deref_mut() {
                    if let Err(source) = ExpansionEngine::apply(backend, &result) {
                        return Err(Box::new(EventError { result, source }));
                    }
                    engine.commit_undo(&result);
                    info!("expansion undone");
                }
            }
        }
        return Ok(());
    }
    // v1.3+ deferred execution: check policy BEFORE executing commands
    let pending = engine.process_deferred(event);
    apply_pending_results(engine, pending, injector, policy, active_backend)
}

/// Apply deferred expansion results with policy pre-approval (v1.3+ architecture).
/// Checks policy before executing commands, preventing irreversible side effects.
fn apply_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    let results = dispatch_pending_results(engine, pending, policy, active_backend);
    apply_results(engine, results, injector, policy, active_backend)
}

/// Apply policy and dispatch deferred expansions without injecting ready results.
///
/// Keeping this stage separate lets evdev perform its physical key-release and
/// input-quiet checks before injection, while still guaranteeing that command
/// expansions are dispatched through the bounded asynchronous worker.
fn dispatch_pending_results(
    engine: &mut ExpansionEngine,
    pending: Vec<wayexpand_core::PendingExpansionResult>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> Vec<ExpansionResult> {
    let mut results = Vec::new();
    for pending_result in pending {
        let has_command = pending_result.command.is_some();

        if let Some(command) = &pending_result.command {
            if let Some(violation) = policy.command_path_violation(&command.program) {
                policy::log_violation(policy, &violation);
                if policy.command_path_is_blocked(&command.program) {
                    engine.restore_deferred_match(&pending_result.matched_text);
                    continue;
                }
            }
        }

        // Check policy BEFORE executing commands
        if policy::check_and_log_expansion_violations(
            policy,
            pending_result.template_text.len(),
            has_command,
            active_backend,
        ) {
            // In safe_mode, block the expansion
            engine.restore_deferred_match(&pending_result.matched_text);
            continue;
        }

        // Policy approved: command-backed expansions go to the bounded worker;
        // static replacements and cache hits are ready immediately.
        let enforcement_policy = policy.effective_enforcement_policy();
        let result = match engine
            .dispatch_pending_with_policy(pending_result, enforcement_policy.max_replacement_size)
        {
            Ok(wayexpand_core::PendingExpansionDispatch::Ready(result)) => result,
            Ok(wayexpand_core::PendingExpansionDispatch::Queued) => continue,
            Err(error) => {
                warn!(%error, "expansion could not be queued or completed");
                continue;
            }
        };
        results.push(result);
    }
    results
}

/// Apply evdev safety gating to expansion results before injection.
/// Ensures physical key-up event is processed and no competing input arrived.
/// Only applies when evdev source is available and in use.
struct EvdevGatingOutcome {
    results: Vec<ExpansionResult>,
    follow_up: Vec<InputEvent>,
    abandoned: Vec<ExpansionResult>,
}

fn apply_evdev_gating(
    mut results: Vec<ExpansionResult>,
    evdev: &mut Option<EvdevSource>,
) -> EvdevGatingOutcome {
    let mut follow_up = Vec::new();
    if let Some(source) = evdev.as_mut() {
        // Wait for physical key release before injecting synthetic input.
        // Injecting while trigger key is held can make synthetic input appear
        // as auto-repeat or cancel the physical release.
        if !evdev_release_is_safe(source.wait_for_key_release(KEY_RELEASE_TIMEOUT)) {
            follow_up = source.take_pending_events();
            return EvdevGatingOutcome {
                results: Vec::new(),
                follow_up,
                abandoned: results,
            };
        }

        // Check if any input arrived during key release wait.
        let input_quiet = match source.wait_for_input_quiet(EVDEV_QUIET_TIMEOUT) {
            Ok(quiet) => quiet,
            Err(error) => {
                warn!(%error, "evdev quiet-period check failed; abandoning expansion");
                // Preserve anything captured before the polling error so the
                // matcher sees the same stream as the focused application.
                follow_up = source.take_pending_events();
                return EvdevGatingOutcome {
                    results: Vec::new(),
                    follow_up,
                    abandoned: results,
                };
            }
        };

        // If other input arrived, handle delimiter preservation or drop expansion
        if !input_quiet {
            follow_up = source.take_pending_events();
            if follow_up.len() == 1 {
                if let InputEvent::Delimiter(character) = &follow_up[0] {
                    // A single delimiter belongs after the complete batch of
                    // results, not after every result in it. Absorb it into
                    // the final erase/reinsert transaction so it is removed
                    // and restored exactly once.
                    absorb_evdev_delimiter(&mut results, *character);
                    // The delimiter is represented in the adjusted result and
                    // must not be replayed a second time through the matcher.
                    follow_up.clear();
                } else {
                    // Other input arrived: don't inject (avoid cursor misplacement)
                    warn!(
                        "input arrived while waiting for key release; dropping expansion to avoid cursor misplacement"
                    );
                    let abandoned = results.clone();
                    results.clear();
                    return EvdevGatingOutcome {
                        results,
                        follow_up,
                        abandoned,
                    };
                }
            } else if follow_up.len() > 1 {
                // Multiple inputs arrived: don't inject
                warn!(
                    "multiple inputs arrived while waiting for key release; dropping expansion to avoid cursor misplacement"
                );
                let abandoned = results.clone();
                results.clear();
                return EvdevGatingOutcome {
                    results,
                    follow_up,
                    abandoned,
                };
            }
        }
    }
    EvdevGatingOutcome {
        results,
        follow_up,
        abandoned: Vec::new(),
    }
}

fn absorb_evdev_delimiter(results: &mut [ExpansionResult], character: char) {
    if let Some(result) = results.last_mut() {
        result.matched_text.push(character);
        result.insert.push(character);
    }
}

/// Replay input captured during evdev gating through the matcher. The focused
/// application receives these events through non-exclusive capture already;
/// this keeps the engine's buffer and boundary state in sync without applying
/// a second replacement for the same physical input.
fn replay_evdev_follow_up(
    engine: &mut ExpansionEngine,
    follow_up: Vec<InputEvent>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for event in follow_up {
        process_event(engine, event, None, policy, active_backend)?;
    }
    Ok(())
}

fn restore_abandoned_results(engine: &mut ExpansionEngine, results: Vec<ExpansionResult>) {
    for result in results {
        engine.restore_deferred_match(&result.matched_text);
    }
}

fn apply_results(
    engine: &mut ExpansionEngine,
    results: Vec<ExpansionResult>,
    mut injector: Option<&mut dyn TextInjector>,
    policy: &wayexpand_core::OrganizationPolicy,
    active_backend: &str,
) -> std::result::Result<(), Box<EventError>> {
    for result in results {
        // Check if expansion violates policy and log if needed
        // Use explicit provenance instead of heuristic: command_backed is set by engine
        if policy::check_and_log_expansion_violations(
            policy,
            result.insert.len(),
            result.command_backed,
            active_backend,
        ) {
            // In safe_mode, block the expansion
            engine.restore_deferred_match(&result.matched_text);
            continue;
        }

        if let Some(backend) = injector.as_deref_mut() {
            // P0 security fix: Never silently switch output transports.
            // If a replacement contains newlines and the selected backend
            // doesn't support them, the expansion will fail with a clear error.
            // This is vastly better than guessing and potentially sending text
            // to an unintended XWayland window.
            if text_contains_newlines(&result.insert) {
                warn!(
                    backend = backend.name(),
                    "expansion contains newlines; selected backend may not support multiline insertion"
                );
            }
            let inject_result = ExpansionEngine::apply(backend, &result);

            if let Err(source) = inject_result {
                engine.restore_deferred_match(&result.matched_text);
                return Err(Box::new(EventError { result, source }));
            }
            engine.commit_applied_expansion(&result);
            info!(
                trigger_chars = result.trigger.chars().count(),
                insert_bytes = result.insert.len(),
                "expansion injected"
            );
        } else {
            engine.restore_deferred_match(&result.matched_text);
            info!(
                trigger_chars = result.trigger.chars().count(),
                matched_chars = result.matched_text.chars().count(),
                insert_bytes = result.insert.len(),
                "expansion matched"
            );
        }
    }
    Ok(())
}

fn parse_args() -> Result<(PathBuf, Option<String>, Option<String>, bool)> {
    let env_path = env::var_os("WAYEXPAND_CONFIG").map(PathBuf::from);
    let mut path = env_path.clone();
    let mut explicit_path = env_path.is_some();
    let mut backend = env::var("WAYEXPAND_BACKEND").ok();
    let mut source = env::var("WAYEXPAND_SOURCE").ok();
    for argument in env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--backend=") {
            backend = Some(value.to_string());
        } else if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_string());
        } else if matches!(argument.as_str(), "--help" | "-h") {
            println!("wayexpand-daemon {}\nusage: wayexpand-daemon [--source=stdin|input-method|evdev] [--backend=none|wlroots|libei] [config]", env!("CARGO_PKG_VERSION"));
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
        !explicit_path,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    use wayexpand_core::{Config, InjectorError};

    #[test]
    fn startup_worker_failure_remains_command_disabled_in_audit_mode() {
        let policy = wayexpand_core::OrganizationPolicy::default();
        assert!(commands_disabled_for_startup(&policy, true));
    }

    #[test]
    fn evdev_release_failure_is_not_safe_to_inject() {
        assert!(!evdev_release_is_safe(Err("device polling failed")));
        assert!(evdev_release_is_safe::<&str>(Ok(())));
    }

    #[test]
    fn startup_command_policy_and_worker_failure_are_combined() {
        let policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..Default::default()
        };
        assert!(commands_disabled_for_startup(&policy, true));
        assert!(commands_disabled_for_startup(&policy, false));
    }

    #[test]
    fn libei_backend_policy_only_blocks_in_safe_mode() {
        let audit = wayexpand_core::OrganizationPolicy {
            safe_mode: false,
            allowed_backends: vec!["input-method-v2".into()],
            ..Default::default()
        };
        assert!(!libei_policy_blocks(&audit));
        let safe = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            ..audit
        };
        assert!(libei_policy_blocks(&safe));
    }

    /// The status body's field set is a documented Stable contract (see
    /// docs/COMPATIBILITY.md, "wayexpand status --json"). This is the
    /// producing side; `status_json_matches_documented_stable_contract` in
    /// the CLI crate's tests is the consuming side, checked against a
    /// fixture with the same field names -- keeping both in sync with the
    /// docs by construction, rather than each drifting independently.
    #[test]
    fn daemon_status_body_matches_documented_stable_contract() {
        let body = status::daemon_status_body(
            "input-method",
            "input-method-v2",
            "connected",
            false,
            Path::new("/home/user/.config/wayexpand/expansions.toml"),
            true,
            CommandMetrics::default(),
        );
        let mut fields: Vec<&str> = body
            .lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key))
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            vec![
                "backend",
                "command_failure_total",
                "command_queue_depth",
                "command_queue_rejected_total",
                "command_timeout_total",
                "config",
                "config_state",
                "paused",
                "source",
                "state",
            ],
            "daemon status body fields no longer match docs/COMPATIBILITY.md's documented Stable contract"
        );
        assert_eq!(
            body,
            "source=input-method\nbackend=input-method-v2\nstate=connected\npaused=false\n\
             config=/home/user/.config/wayexpand/expansions.toml\nconfig_state=ok\n\
             command_queue_depth=0\ncommand_queue_rejected_total=0\n\
             command_timeout_total=0\ncommand_failure_total=0"
        );
    }

    struct RecordingInjector {
        calls: Vec<String>,
    }

    impl TextInjector for RecordingInjector {
        fn name(&self) -> &'static str {
            "test"
        }

        fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
            self.calls.push(format!("erase:{trigger}"));
            Ok(())
        }

        fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
            self.calls.push(format!("insert:{text}"));
            Ok(())
        }
    }

    struct FailingInjector;

    impl TextInjector for FailingInjector {
        fn name(&self) -> &'static str {
            "failing-test"
        }

        fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
            Err(InjectorError {
                backend: "failing-test",
                message: "connection lost".into(),
                retryable: true,
            })
        }

        fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
            unreachable!("erase fails first")
        }
    }

    #[test]
    fn process_event_applies_all_matches_in_order() {
        let config = Config::parse(
            r#"
                [[expansion]]
                trigger = ":a"
                replacement = "alpha"

                [[expansion]]
                trigger = ":b"
                replacement = "beta"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let mut injector = RecordingInjector { calls: Vec::new() };
        let policy = wayexpand_core::OrganizationPolicy::default();
        process_event(
            &mut engine,
            InputEvent::Text(":a:b".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        assert_eq!(
            injector.calls,
            ["erase::a", "insert:alpha", "erase::b", "insert:beta"]
        );
    }

    #[test]
    fn evdev_follow_up_is_replayed_in_order_to_the_matcher() {
        let config = Config::parse(
            r#"
                [[expansion]]
                trigger = ":sigx "
                replacement = "ok"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let policy = wayexpand_core::OrganizationPolicy::default();

        process_event(
            &mut engine,
            InputEvent::Text(":sig".into()),
            None,
            &policy,
            "libei",
        )
        .unwrap();
        replay_evdev_follow_up(
            &mut engine,
            vec![InputEvent::Text("x".into())],
            &policy,
            "libei",
        )
        .unwrap();

        let mut injector = RecordingInjector { calls: Vec::new() };
        process_event(
            &mut engine,
            InputEvent::Delimiter(' '),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        assert_eq!(injector.calls, ["erase::sigx ", "insert:ok"]);
    }

    #[test]
    fn evdev_delimiter_is_absorbed_only_by_the_final_result() {
        let mut results = vec![
            ExpansionResult {
                trigger: ":a".into(),
                matched_text: ":a".into(),
                insert: "alpha".into(),
                cursor_offset: None,
                reinsert_after: None,
                command_backed: false,
                undoable: true,
            },
            ExpansionResult {
                trigger: ":b".into(),
                matched_text: ":b".into(),
                insert: "beta".into(),
                cursor_offset: None,
                reinsert_after: None,
                command_backed: false,
                undoable: true,
            },
        ];

        absorb_evdev_delimiter(&mut results, ' ');

        assert_eq!(results[0].matched_text, ":a");
        assert_eq!(results[0].insert, "alpha");
        assert_eq!(results[1].matched_text, ":b ");
        assert_eq!(results[1].insert, "beta ");
    }

    #[cfg(unix)]
    #[test]
    fn deferred_command_output_over_organization_limit_is_not_injected() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-daemon-policy-output-{}",
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
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.enable_async_commands());
        let mut injector = RecordingInjector { calls: Vec::new() };
        let policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            max_replacement_size: 256,
            ..wayexpand_core::OrganizationPolicy::default()
        };

        process_event(
            &mut engine,
            InputEvent::Text(":large".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let completed = engine.drain_completed_commands();
            for result in &completed {
                ExpansionEngine::apply(&mut injector, result).unwrap();
            }
            let metrics = engine.command_metrics();
            if marker.exists() && metrics.command_queue_depth == 0 && metrics.command_in_flight == 0
            {
                for _ in 0..5 {
                    let _ = engine.drain_completed_commands();
                    thread::sleep(Duration::from_millis(2));
                }
                break;
            }
            assert!(Instant::now() < deadline, "the subprocess should complete");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            injector.calls.is_empty(),
            "oversized output must not be injected"
        );
        let _ = std::fs::remove_file(marker);
    }

    #[cfg(unix)]
    #[test]
    fn deferred_command_does_not_block_daemon_input_processing() {
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
        let mut engine = ExpansionEngine::new(config).unwrap();
        assert!(engine.enable_async_commands());
        let mut injector = RecordingInjector { calls: Vec::new() };
        let policy = wayexpand_core::OrganizationPolicy::default();
        let started = Instant::now();
        process_event(
            &mut engine,
            InputEvent::Text(":slow".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "daemon input processing waited for the child process"
        );
        assert!(injector.calls.is_empty());

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let results = engine.drain_completed_commands();
            if !results.is_empty() {
                for result in &results {
                    ExpansionEngine::apply(&mut injector, result).unwrap();
                }
                break;
            }
            assert!(
                Instant::now() < deadline,
                "command completion was not drained"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(injector.calls, ["erase::slow", "insert:done"]);
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        let initial = Duration::from_millis(250);
        assert_eq!(next_retry_delay(initial), Duration::from_millis(500));
        assert_eq!(
            next_retry_delay(Duration::from_secs(20)),
            Duration::from_secs(30)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn unsupported_output_backend_fails_without_retry() {
        let error = match connect_output_backend("unknown", true, None) {
            Ok(_) => panic!("unknown backend unexpectedly connected"),
            Err(error) => error,
        };
        assert!(!error.retryable);
        assert!(error.message.contains("unknown output backend"));
    }

    #[test]
    fn bounded_line_reader_matches_lines_semantics() {
        let mut reader = BufReader::new(Cursor::new(b"first\r\nsecond\n"));
        assert_eq!(
            read_bounded_line(&mut reader).unwrap().as_deref(),
            Some("first")
        );
        assert_eq!(
            read_bounded_line(&mut reader).unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
    }

    #[test]
    fn oversized_and_invalid_lines_are_rejected_without_unbounded_allocation() {
        let oversized = vec![b'a'; MAX_STDIN_LINE_BYTES + 1];
        let mut reader = BufReader::new(Cursor::new(oversized));
        let error = read_bounded_line(&mut reader).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let mut reader = BufReader::new(Cursor::new(vec![0xff, b'\n']));
        let error = read_bounded_line(&mut reader).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn stdin_queue_applies_backpressure_at_a_bounded_capacity() {
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_INPUT_LINES);
        for _ in 0..MAX_PENDING_INPUT_LINES {
            sender.try_send(String::from("line")).unwrap();
        }
        assert!(matches!(
            sender.try_send(String::from("overflow")),
            Err(mpsc::TrySendError::Full(_))
        ));
        drop(receiver);
    }

    #[test]
    fn process_event_can_match_without_an_injector() {
        let config = Config::parse(
            r#"[[expansion]]
            trigger = ":x"
            replacement = "ok""#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let policy = wayexpand_core::OrganizationPolicy::default();
        process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            None,
            &policy,
            "libei",
        )
        .unwrap();
    }

    #[test]
    fn undo_is_not_consumed_when_injector_is_unavailable() {
        let config = Config::parse(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[expansion]]
            trigger = ":x"
            replacement = "ok"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let policy = wayexpand_core::OrganizationPolicy::default();
        let mut injector = RecordingInjector { calls: Vec::new() };

        process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            None,
            &policy,
            "libei",
        )
        .unwrap();
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        assert_eq!(
            injector.calls,
            ["erase::x", "insert:ok", "erase:ok", "insert::x"]
        );
    }

    #[test]
    fn undo_is_not_consumed_when_injection_fails() {
        let config = Config::parse(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[expansion]]
            trigger = ":x"
            replacement = "ok"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let policy = wayexpand_core::OrganizationPolicy::default();
        let mut initial_injector = RecordingInjector { calls: Vec::new() };
        process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut initial_injector),
            &policy,
            "libei",
        )
        .unwrap();

        let error = process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            Some(&mut FailingInjector),
            &policy,
            "libei",
        )
        .unwrap_err();
        assert!(error.retryable());

        let mut retry_injector = RecordingInjector { calls: Vec::new() };
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            Some(&mut retry_injector),
            &policy,
            "libei",
        )
        .unwrap();
        assert_eq!(retry_injector.calls, ["erase:ok", "insert::x"]);
    }

    #[test]
    fn disabling_hotkeys_does_not_disable_undo_or_ordinary_keys() {
        let config = Config::parse(
            r#"
            [settings]
            undo_chord = "Ctrl+Z"

            [[expansion]]
            trigger = ":x"
            replacement = "ok"
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let mut injector = RecordingInjector { calls: Vec::new() };
        let policy = wayexpand_core::OrganizationPolicy {
            safe_mode: true,
            disable_hotkeys: true,
            ..Default::default()
        };

        process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("A").unwrap()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        assert_eq!(
            injector.calls,
            ["erase::x", "insert:ok", "erase:ok", "insert::x"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn deferred_command_without_worker_never_runs_synchronously() {
        let marker = std::env::temp_dir().join(format!(
            "wayexpand-daemon-no-sync-command-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let config = Config::parse(&format!(
            r#"
            [[expansion]]
            trigger = ":command"
            replacement = ""
            [expansion.command]
            program = "/bin/sh"
            args = ["-c", "printf ran > '{marker}'"]
            timeout_ms = 1000
            "#,
            marker = marker.display()
        ))
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let mut injector = RecordingInjector { calls: Vec::new() };
        let policy = wayexpand_core::OrganizationPolicy::default();

        process_event(
            &mut engine,
            InputEvent::Text(":command".into()),
            Some(&mut injector),
            &policy,
            "evdev",
        )
        .unwrap();

        assert!(!marker.exists());
        assert!(injector.calls.is_empty());
        assert_eq!(engine.command_metrics().command_in_flight, 0);
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn process_event_queues_hotkeys_without_waiting_for_the_child() {
        let config = Config::parse(
            r#"
                [[hotkey]]
                chord = "Ctrl+M"
                [hotkey.command]
                program = "/bin/sh"
                args = ["-c", "sleep 0.1"]
                timeout_ms = 500
            "#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        engine.enable_async_commands();
        let policy = wayexpand_core::OrganizationPolicy::default();
        let started = Instant::now();
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+M").unwrap()),
            None,
            &policy,
            "libei",
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(50));

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some((_, result)) = engine.drain_completed_hotkeys().pop() {
                assert!(result.is_ok());
                break;
            }
            assert!(Instant::now() < deadline, "hotkey did not complete");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn retryable_injection_failure_preserves_ambiguous_result() {
        let config = Config::parse(
            r#"[[expansion]]
            trigger = ":x"
            replacement = "ok""#,
        )
        .unwrap();
        let mut engine = ExpansionEngine::new(config).unwrap();
        let policy = wayexpand_core::OrganizationPolicy::default();
        let error = process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut FailingInjector),
            &policy,
            "libei",
        )
        .unwrap_err();
        assert!(error.retryable());
        assert_eq!(error.result.trigger, ":x");
        assert_eq!(error.result.insert, "ok");
    }
}
