mod args;
mod backend_lifecycle;
mod build_info;
mod clipboard;
mod control;
mod events;
mod focus;
mod input_loop;
mod latency;
mod output_loop;
mod policy;
mod reactor;
mod reload;
mod source_steps;
mod status;
mod status_publisher;
mod turn;
mod usage;
mod waker;

use anyhow::Result;
use args::parse_args;
use control::{ControlServer, FocusSnapshot};
use events::{
    apply_evdev_gating, apply_results, dispatch_pending_results, process_event,
    replay_evdev_follow_up, restore_abandoned_results, EvdevGatingOutcome,
};
use focus::{drain_pending_window_events, focus_token, publish_focus_snapshot, FocusState};
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
use source_steps::Step;
use status_publisher::{
    set_daemon_status, set_daemon_status_with_runtime_capabilities, StatusPublisher,
};
use std::{
    env,
    io::{self, BufRead},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
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
    MAX_WINDOW_INSTANCE_ID_BYTES,
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

/// State of the reactor loop: fixed deployment context followed by the
/// runtime state each turn reads and updates.
struct Daemon {
    control: ControlServer,
    policy: wayexpand_core::OrganizationPolicy,
    path: PathBuf,
    portal_token_path: Option<PathBuf>,
    window_tracker: Option<backend_lifecycle::WindowTrackerHandle>,
    active_source: &'static str,
    active_backend: &'static str,
    backend_name: &'static str,
    managed: bool,
    input_method_mode: bool,
    evdev_mode: bool,
    config: ReloadableConfig,
    injector: Option<Box<dyn TextInjector>>,
    input_method: Option<wayexpand_backend_input_method::InputMethodSource>,
    evdev: Option<EvdevSource>,
    receiver: Option<mpsc::Receiver<String>>,
    status_publisher: StatusPublisher,
    focus_state: FocusState,
    connection_state: &'static str,
    paused: bool,
    reconnect_delay: Duration,
    output_retry_at: Option<Instant>,
    output_retry_delay: Duration,
    output_failures: Option<mpsc::Receiver<output_loop::OutputFailure>>,
    stdin_closed: bool,
    logged_queue_rejections: u64,
    waker: waker::Waker,
    usage: usage::UsageRecorder,
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
    // Work arriving from other threads wakes the reactor instead of waiting
    // for its next poll timeout.
    let waker = waker::Waker::new()
        .map_err(|error| anyhow::anyhow!("could not create the reactor wakeup: {error}"))?;
    control.set_waker(waker.clone());
    config
        .engine
        .set_clipboard_reader(Some(clipboard::wl_paste_reader()));
    let completion_waker = waker.clone();
    config
        .engine
        .set_completion_notifier(Some(Arc::new(move || completion_waker.wake())));
    let signal_stop = control.stop_requested.clone();
    let signal_waker = waker.clone();
    let mut signals = Signals::new([SIGINT, SIGTERM])
        .map_err(|error| anyhow::anyhow!("could not install signal handlers: {error}"))?;
    thread::spawn(move || {
        if signals.forever().next().is_some() {
            signal_stop.store(true, std::sync::atomic::Ordering::Release);
            signal_waker.wake();
        }
    });
    info!(path = %config.path().display(), fleet = use_fleet, "configuration loaded");
    if let Some(socket) = control.path() {
        info!(path = %socket.display(), "control socket ready");
    } else {
        warn!("XDG_RUNTIME_DIR unavailable; control socket disabled");
    }
    let portal_token_path = portal_token_path();
    let input_method = match source_name {
        "input-method" => {
            let mut source = connect_input_method_with_retry(
                &control,
                &path,
                config.healthy(),
                config.engine.libei_token_persistence(),
                portal_token_path.as_deref(),
                &policy,
            )?;
            source.set_wake_fd(Some(waker.fd()));
            Some(source)
        }
        _ => None,
    };
    let evdev = match source_name {
        "evdev" => {
            let mut source =
                connect_evdev_with_retry(&control, &path, backend_name, config.healthy())?;
            source.set_wake_fd(Some(waker.fd()));
            Some(source)
        }
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
    let reconnect_delay = Duration::from_millis(250);
    let output_retry_at: Option<Instant> = None;
    let output_retry_delay = Duration::from_millis(250);
    let mut output_failures = None;
    let injector: Option<Box<dyn TextInjector>> = if input_method.is_some() {
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
    let connection_state = if input_method_mode || evdev_mode {
        "connected"
    } else {
        "running"
    };
    let paused = false;
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
    let focus_state = FocusState::new(config.engine.current_window().cloned());
    publish_focus_snapshot(&control, &focus_state);

    let stdin_closed = false;
    let logged_queue_rejections = 0;
    let usage = usage::UsageRecorder::new(wayexpand_core::usage_stats_path(&path));
    let mut daemon = Daemon {
        control,
        policy,
        path,
        portal_token_path,
        window_tracker,
        active_source,
        active_backend,
        backend_name,
        managed,
        input_method_mode,
        evdev_mode,
        config,
        injector,
        input_method,
        evdev,
        receiver,
        status_publisher,
        focus_state,
        connection_state,
        paused,
        reconnect_delay,
        output_retry_at,
        output_retry_delay,
        output_failures,
        stdin_closed,
        logged_queue_rejections,
        waker,
        usage,
    };
    loop {
        let metrics = match daemon.maintain()? {
            std::ops::ControlFlow::Continue(metrics) => metrics,
            std::ops::ControlFlow::Break(()) => break,
        };
        // Capture sources are woken by the waker when commands finish; only
        // the stdin stream (a channel, not a descriptor) still polls for them.
        let poll_interval = if daemon.receiver.is_some() {
            input_poll_interval(metrics)
        } else {
            input_loop::IDLE_MAINTENANCE_INTERVAL
        };
        daemon.apply_completed_commands()?;
        daemon.answer_explain_request();
        if daemon.handle_insert_request()? {
            continue;
        }
        daemon.publish_status(metrics);
        daemon.recover_output()?;
        if daemon.input_method_mode {
            match daemon.input_method_step(poll_interval)? {
                Step::Next => continue,
                Step::Stop => break,
            }
        }
        if daemon.evdev_mode {
            match daemon.evdev_step(poll_interval)? {
                Step::Next => continue,
                Step::Stop => break,
            }
        }
        match daemon.stdin_step(poll_interval)? {
            Step::Next => continue,
            Step::Stop => break,
        }
    }
    warn!("input stream ended; daemon stopping");
    daemon.usage.collect(&mut daemon.config.engine);
    daemon.usage.flush();
    // Backends get an explicit teardown opportunity. The bounded wait keeps
    // systemd stop independent from a broken portal implementation, while
    // still allowing libei to close its portal session and Tokio runtime
    // cleanly in the normal case.
    if let Some(injector) = daemon.injector.take() {
        shutdown_injector(injector);
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

#[cfg(test)]
mod tests;
