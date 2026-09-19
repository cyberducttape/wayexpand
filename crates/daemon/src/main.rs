mod control;
mod reload;

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
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_backend_libei::LibeiInjector;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_backend_wlroots_toplevel::WlrootsToplevelTracker;
use wayexpand_core::{
    default_config_path, ExpansionEngine, ExpansionError, ExpansionResult, InputEvent,
    TextInjector, WindowContext, WindowTracker,
};

/// How long to wait for physically held keys to be released before injecting
/// an expansion in evdev mode. Generous enough to cover a deliberate
/// keypress, bounded so a genuinely held key cannot stall expansion.
const KEY_RELEASE_TIMEOUT: Duration = Duration::from_millis(400);
const MAX_STDIN_LINE_BYTES: usize = 1024 * 1024;
const MAX_PENDING_INPUT_LINES: usize = 64;

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
    let (path, source_name, backend_name) = parse_args()?;
    let mut config = ReloadableConfig::load(&path).map_err(|_| {
        anyhow::anyhow!(
            "could not load configuration {}; run `wayexpand doctor` for details",
            path.display()
        )
    })?;
    config.engine.enable_async_commands();
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
    info!(path = %config.path().display(), "configuration loaded");
    if let Some(socket) = control.path() {
        info!(path = %socket.display(), "control socket ready");
    } else {
        warn!("XDG_RUNTIME_DIR unavailable; control socket disabled");
    }
    let mut input_method = match source_name.as_deref() {
        Some("input-method") => {
            if backend_name.is_some() && backend_name.as_deref() != Some("none") {
                anyhow::bail!(
                    "input-method source is also its injector; do not combine it with an output backend"
                );
            }
            Some(connect_input_method_with_retry(&control, &path)?)
        }
        _ => None,
    };
    let mut evdev = match source_name.as_deref() {
        Some("evdev") => Some(connect_evdev_with_retry(
            &control,
            &path,
            backend_name.as_deref(),
            config.healthy(),
        )?),
        _ => None,
    };
    match source_name.as_deref() {
        None | Some("stdin") | Some("input-method") | Some("evdev") => {}
        Some(other) => {
            anyhow::bail!("unknown source {other:?}; expected stdin, input-method, or evdev")
        }
    }
    let input_method_mode = source_name.as_deref() == Some("input-method");
    if input_method_mode {
        warn!(
            "input-method-v2 backend selected: exclusive keyboard capture is active; \
            unsupported keys (Escape, arrows, F-keys, etc.) will not pass through. \
            Use libei or wlroots backend for full key support."
        );
    }
    let evdev_mode = source_name.as_deref() == Some("evdev");
    let active_source = source_name.as_deref().unwrap_or("stdin");
    let mut reconnect_delay = Duration::from_millis(250);
    let mut injector: Option<Box<dyn TextInjector>> = if input_method.is_some() {
        None
    } else {
        match backend_name.as_deref() {
            None | Some("none") => None,
            Some(backend @ ("wlroots" | "libei")) => {
                let Some(injector) = connect_output_with_retry(
                    &control,
                    active_source,
                    backend,
                    &path,
                    config.healthy(),
                )?
                else {
                    anyhow::bail!("output backend startup cancelled while stopping")
                };
                Some(injector)
            }
            Some(other) => {
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
    set_daemon_status(
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
    loop {
        if let Some(receiver) = window_tracker.as_ref() {
            let mut latest = None;
            while let Ok(window) = receiver.try_recv() {
                latest = Some(window);
            }
            if let Some(window) = latest {
                process_event(&mut config.engine, InputEvent::WindowChanged(window), None)?;
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
        let completed_commands = config.engine.drain_completed_commands();
        if !completed_commands.is_empty() {
            if input_method_mode {
                if let Some(source) = input_method.as_mut() {
                    apply_results(completed_commands, Some(source))?;
                }
            } else if let Some(mut backend) = injector.take() {
                let result = apply_results(completed_commands, Some(backend.as_mut()));
                injector = Some(backend);
                result?;
            } else {
                apply_results(completed_commands, None)?;
            }
        }
        set_daemon_status(
            &control,
            active_source,
            active_backend,
            connection_state,
            &path,
            config.healthy(),
        );
        if input_method_mode {
            if input_method.is_none() {
                match InputMethodSource::connect() {
                    Ok(source) => {
                        input_method = Some(source);
                        reconnect_delay = Duration::from_millis(250);
                        connection_state = "connected";
                        set_daemon_status(
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
            let event_result = source.next_event_timeout(Duration::from_millis(250));
            match event_result {
                Ok(Some(event)) => {
                    drain_pending_window_events(&window_tracker, &mut config.engine)?;
                    let result = match input_method.as_mut() {
                        Some(source) => process_event(&mut config.engine, event, Some(source)),
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
                            )?;
                            set_daemon_status(
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
                    )?;
                    set_daemon_status(
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
            let event_result = source.next_event_timeout(Duration::from_millis(250));
            match event_result {
                Ok(Some(event)) => {
                    drain_pending_window_events(&window_tracker, &mut config.engine)?;
                    let result = if let Some(mut backend) = injector.take() {
                        // Capture is non-exclusive and a match fires on
                        // key-down, so the trigger's last key is still held
                        // right now. Injecting before it comes up makes the
                        // compositor treat our duplicate press as auto-repeat
                        // and our release as cancelling the physical one,
                        // eating exactly those characters.
                        if let Some(source) = evdev.as_mut() {
                            if let Err(error) = source.wait_for_key_release(KEY_RELEASE_TIMEOUT) {
                                warn!(%error, "waiting for key release failed; injecting anyway");
                            }
                        }
                        let result =
                            process_event(&mut config.engine, event, Some(backend.as_mut()));
                        injector = Some(backend);
                        result
                    } else {
                        process_event(&mut config.engine, event, None)
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
                            config.engine.process(InputEvent::Boundary);
                            connection_state = "reconnecting";
                            set_daemon_status(
                                &control,
                                active_source,
                                active_backend,
                                connection_state,
                                &path,
                                config.healthy(),
                            );
                            let backend = backend_name.as_deref().unwrap_or("none");
                            let Some(reconnected) = connect_output_with_retry(
                                &control,
                                active_source,
                                backend,
                                &path,
                                config.healthy(),
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
                    config.engine.process(InputEvent::Boundary);
                    set_daemon_status(
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
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        let Some(receiver) = receiver.as_ref() else {
            break;
        };
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                drain_pending_window_events(&window_tracker, &mut config.engine)?;
                if injector.is_some() {
                    for character in line.chars() {
                        let event = InputEvent::Text(character.to_string());
                        let (result, backend) = if let Some(mut backend) = injector.take() {
                            let result =
                                process_event(&mut config.engine, event, Some(backend.as_mut()));
                            (result, Some(backend))
                        } else {
                            (process_event(&mut config.engine, event, None), None)
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
                            config.engine.process(InputEvent::Boundary);
                            connection_state = "reconnecting";
                            set_daemon_status(
                                &control,
                                active_source,
                                backend_name.as_deref().unwrap_or("none"),
                                connection_state,
                                &path,
                                config.healthy(),
                            );
                            let backend_name = backend_name.as_deref().unwrap_or("none");
                            let Some(reconnected) = connect_output_with_retry(
                                &control,
                                active_source,
                                backend_name,
                                &path,
                                config.healthy(),
                            )?
                            else {
                                break;
                            };
                            injector = Some(reconnected);
                            connection_state = "connected";
                        }
                    }
                    if let Some(backend) = injector.as_deref_mut() {
                        process_event(&mut config.engine, InputEvent::Boundary, Some(backend))?;
                    }
                } else {
                    process_event(&mut config.engine, InputEvent::Text(line), None)?;
                    process_event(&mut config.engine, InputEvent::Boundary, None)?;
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
/// tracking is inherently compositor-specific. Today, KDE Plasma (KWin) and
/// wlroots-based compositors (Sway, Hyprland, river) are implemented.
/// `app_filter`-scoped expansions simply fail closed everywhere else, exactly
/// as they would if this thread were never started.
fn spawn_window_tracker() -> Option<mpsc::Receiver<Option<WindowContext>>> {
    // Phase 3: Try backends in order of preference
    // 1. KDE Plasma (KWin D-Bus) - most reliable
    // 2. wlroots (foreign-toplevel-management-v1) - Sway/Hyprland/river

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

    // Try wlroots (Sway, Hyprland, river, etc.)
    match WlrootsToplevelTracker::new(Duration::from_secs(3)) {
        Ok(_) => {
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let mut tracker = match WlrootsToplevelTracker::new(Duration::from_secs(3)) {
                    Ok(tracker) => tracker,
                    Err(error) => {
                        warn!(%error, "wlroots window tracker failed to start after probe succeeded");
                        return;
                    }
                };
                info!("window tracker active (wlr-foreign-toplevel-management-v1)");
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
                            warn!(%error, "wlroots window tracker stopped");
                            break;
                        }
                    }
                }
            });
            Some(receiver)
        }
        Err(_) => {
            info!("window tracking unavailable; app_filter-scoped expansions will not match");
            None
        }
    }
}

/// Drain any pending window-change events from the tracker's receiver
/// and apply them to the engine. This prevents app-filter races where a
/// focus change arrives between input-event wait and processing.
fn drain_pending_window_events(
    window_tracker: &Option<mpsc::Receiver<Option<WindowContext>>>,
    engine: &mut ExpansionEngine,
) -> Result<()> {
    if let Some(receiver) = window_tracker.as_ref() {
        let mut latest = None;
        while let Ok(window) = receiver.try_recv() {
            latest = Some(window);
        }
        if let Some(window) = latest {
            process_event(engine, InputEvent::WindowChanged(window), None)?;
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

/// Builds the control socket's `key=value` status body (everything the
/// running daemon reports about itself via `status`/`status --json`).
/// Pulled out of `set_daemon_status` as a pure function so its exact field
/// set can be unit-tested directly -- this is the other half of the
/// contract docs/COMPATIBILITY.md documents as "Stable" for
/// `wayexpand status --json`; `status_json_matches_documented_stable_contract`
/// in the CLI crate tests the parsing side against the same field names.
fn daemon_status_body(
    source: &str,
    backend: &str,
    state: &str,
    paused: bool,
    config_path: &Path,
    config_healthy: bool,
) -> String {
    format!(
        "source={source}\nbackend={backend}\nstate={state}\npaused={paused}\nconfig={}\nconfig_state={}",
        config_path.display(),
        if config_healthy {
            "ok"
        } else {
            "reload-rejected"
        }
    )
}

fn set_daemon_status(
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    state: &str,
    config_path: &Path,
    config_healthy: bool,
) {
    control.set_status(daemon_status_body(
        source,
        backend,
        state,
        control
            .pause_requested
            .load(std::sync::atomic::Ordering::Acquire),
        config_path,
        config_healthy,
    ));
}

fn connect_input_method_with_retry(
    control: &control::ControlServer,
    config_path: &Path,
) -> Result<InputMethodSource> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match InputMethodSource::connect() {
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
    backend_name: Option<&str>,
    config_healthy: bool,
) -> Result<EvdevSource> {
    let backend = backend_name.unwrap_or("none");
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match EvdevSource::connect() {
            Ok(source) => {
                set_daemon_status(
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
                set_daemon_status(
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
) -> std::result::Result<Box<dyn TextInjector>, OutputConnectError> {
    match name {
        "wlroots" => WlrootsInjector::connect()
            .map(|injector| Box::new(injector) as Box<dyn TextInjector>)
            .map_err(|error| OutputConnectError {
                retryable: error.is_retryable(),
                message: format!("connecting wlroots output backend: {error}"),
            }),
        "libei" => LibeiInjector::connect()
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
) -> Result<Option<Box<dyn TextInjector>>> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match connect_output_backend(backend) {
            Ok(injector) => {
                set_daemon_status(
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
                set_daemon_status(
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
) -> std::result::Result<(), Box<EventError>> {
    if let InputEvent::Key(chord) = event {
        for action in engine.process_key(&chord) {
            match ExpansionEngine::execute_hotkey(&action) {
                Ok(()) => info!(chord = %action.chord, "hotkey action completed"),
                Err(error) => warn!(chord = %action.chord, %error, "hotkey action failed"),
            }
        }
        if let Some(result) = engine.try_undo(&chord) {
            if let Some(backend) = injector.as_deref_mut() {
                if let Err(source) = ExpansionEngine::apply(backend, &result) {
                    return Err(Box::new(EventError { result, source }));
                }
                info!("expansion undone");
            }
        }
        return Ok(());
    }
    apply_results(engine.process(event), injector)
}

fn apply_results(
    results: Vec<ExpansionResult>,
    mut injector: Option<&mut dyn TextInjector>,
) -> std::result::Result<(), Box<EventError>> {
    for result in results {
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
                return Err(Box::new(EventError { result, source }));
            }
            info!(
                trigger_chars = result.trigger.chars().count(),
                insert_bytes = result.insert.len(),
                "expansion injected"
            );
        } else {
            info!(
                trigger_chars = result.trigger.chars().count(),
                erase_chars = result.erase_chars,
                insert_bytes = result.insert.len(),
                "expansion matched"
            );
        }
    }
    Ok(())
}

fn parse_args() -> Result<(PathBuf, Option<String>, Option<String>)> {
    let mut path = env::var_os("WAYEXPAND_CONFIG").map(PathBuf::from);
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
        }
    }
    Ok((path.unwrap_or_else(default_config_path), source, backend))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    use wayexpand_core::{Config, InjectorError};

    /// The status body's field set is a documented Stable contract (see
    /// docs/COMPATIBILITY.md, "wayexpand status --json"). This is the
    /// producing side; `status_json_matches_documented_stable_contract` in
    /// the CLI crate's tests is the consuming side, checked against a
    /// fixture with the same field names -- keeping both in sync with the
    /// docs by construction, rather than each drifting independently.
    #[test]
    fn daemon_status_body_matches_documented_stable_contract() {
        let body = daemon_status_body(
            "input-method",
            "input-method-v2",
            "connected",
            false,
            Path::new("/home/user/.config/wayexpand/expansions.toml"),
            true,
        );
        let mut fields: Vec<&str> = body
            .lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key))
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            vec!["backend", "config", "config_state", "paused", "source", "state"],
            "daemon status body fields no longer match docs/COMPATIBILITY.md's documented Stable contract"
        );
        assert_eq!(
            body,
            "source=input-method\nbackend=input-method-v2\nstate=connected\npaused=false\n\
             config=/home/user/.config/wayexpand/expansions.toml\nconfig_state=ok"
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
        process_event(
            &mut engine,
            InputEvent::Text(":a:b".into()),
            Some(&mut injector),
        )
        .unwrap();
        assert_eq!(
            injector.calls,
            ["erase::a", "insert:alpha", "erase::b", "insert:beta"]
        );
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
        let error = match connect_output_backend("unknown") {
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
        process_event(&mut engine, InputEvent::Text(":x".into()), None).unwrap();
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
        let error = process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut FailingInjector),
        )
        .unwrap_err();
        assert!(error.retryable());
        assert_eq!(error.result.trigger, ":x");
        assert_eq!(error.result.insert, "ok");
    }
}
