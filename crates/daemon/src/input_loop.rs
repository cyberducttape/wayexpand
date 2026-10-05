//! Input source connection and reconnection logic.
//!
//! Handles:
//! - Input-method-v2 backend with libei key pass-through
//! - Evdev source connection with device discovery
//! - Exponential backoff retry logic
//! - Adaptive polling intervals based on command queue depth

use anyhow::Result;
use std::{
    io,
    path::Path,
    sync::atomic::AtomicBool,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_evdev::EvdevSource;
use wayexpand_backend_input_method::{InputMethodError, InputMethodSource};
use wayexpand_core::{CommandMetrics, OrganizationPolicy, TextInjector};

/// Low-latency polling while command/hotkey work is queued or running.
const ACTIVE_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// The stdin test/pipe source uses `recv_timeout`, which cannot be woken by
/// the control socket. Keep control requests responsive without a 250 ms wait.
const IDLE_STDIN_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Idle maintenance cadence for reload/pause/stop checks.
pub(crate) const IDLE_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);

pub use crate::output_loop::connect_output_backend;
use crate::{control, status};

/// Start the stdin input source used by the testable/pipe-driven daemon mode.
///
/// Reading is isolated from the reactor because stdin has no useful polling
/// timeout on all supported platforms. The bounded channel keeps a producer
/// that writes faster than the matcher from consuming unbounded memory.
pub fn spawn_stdin_reader() -> Result<mpsc::Receiver<String>> {
    let (sender, receiver) = mpsc::sync_channel(crate::MAX_PENDING_INPUT_LINES);
    thread::Builder::new()
        .name("wayexpand-stdin-reader".into())
        .spawn(move || {
            let mut reader = io::BufReader::new(io::stdin().lock());
            loop {
                match crate::read_bounded_line(&mut reader) {
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
        })
        .map_err(|error| anyhow::anyhow!("could not start stdin reader: {error}"))?;
    Ok(receiver)
}

/// Determine polling interval based on async work queue depth.
pub fn input_poll_interval(metrics: CommandMetrics) -> Duration {
    if metrics.command_queue_depth > 0 || metrics.command_in_flight > 0 {
        ACTIVE_COMPLETION_POLL_INTERVAL
    } else {
        IDLE_STDIN_POLL_INTERVAL
    }
}

/// Connect to input-method-v2 with mandatory libei key pass-through.
///
/// The input-method-v2 source may receive an exclusive keyboard grab. It must
/// therefore never be created successfully without a working pass-through
/// injector: a retryable libei outage means no input-method source exists and
/// normal keyboard events remain with the compositor/application.
pub fn connect_input_method_session(
    control: &control::ControlServer,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
    policy: &OrganizationPolicy,
) -> Result<InputMethodSource, InputMethodError> {
    if !policy.backend_allowed("libei") {
        let violation = format!(
            "backend 'libei' is not in allowed list: {:?}",
            policy.allowed_backends
        );
        crate::policy::log_violation(policy, &violation);
        if libei_policy_blocks(policy) {
            return Err(InputMethodError::Protocol(
                "organization policy prohibits input-method-v2 because libei key ".to_owned()
                    + "pass-through is mandatory for safe keyboard capture",
            ));
        }
        info!("audit mode permits libei key pass-through with a disallowed backend");
    }

    // Establish pass-through before creating the input-method source. This
    // ordering is the keyboard-safety boundary: once the source exists, the
    // compositor is allowed to grant it an exclusive keyboard grab.
    let key_injector = match connect_output_backend(
        "libei",
        persist_portal_token,
        portal_token_path,
    ) {
        Ok(key_injector) => key_injector,
        Err(error) if error.retryable => {
            warn!(%error, "libei unavailable; input-method capture will remain disabled until it recovers");
            status::set_daemon_status_direct(
                control,
                "input-method",
                "libei",
                "reconnecting",
                config_path,
                config_healthy,
            );
            return Err(InputMethodError::PassThrough {
                message: error.message,
                retryable: true,
            });
        }
        Err(error) => {
            return Err(InputMethodError::Protocol(format!(
                "libei unavailable: {}",
                error.message
            )));
        }
    };

    let backend_mode = key_injector.status_detail();
    let source = InputMethodSource::connect()?.with_key_pass_through(key_injector);
    status::set_daemon_status_with_mode(
        control,
        "input-method",
        "input-method-v2",
        "connected",
        config_path,
        config_healthy,
        backend_mode,
        CommandMetrics::default(),
    );
    Ok(source)
}

/// Check if libei is blocked by policy.
pub fn libei_policy_blocks(policy: &OrganizationPolicy) -> bool {
    policy.safe_mode && !policy.backend_allowed("libei")
}

/// Connect to input-method-v2 with exponential backoff retry.
pub fn connect_input_method_with_retry(
    control: &control::ControlServer,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
    policy: &OrganizationPolicy,
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
                return Ok(source);
            }
            Err(error) if error.is_retryable() => {
                warn!(%error, ?retry_delay, "input-method unavailable at startup; retrying");
                // Build this through the shared status writer like every other
                // transition. Hand-formatting it here dropped `backend_mode`
                // and all nine command-metric fields from the documented
                // control-socket contract, and asserted `paused=false` and
                // `config_state=ok` regardless of whether the user had paused
                // or the last reload had been rejected.
                status::set_daemon_status_direct(
                    control,
                    "input-method",
                    "input-method-v2",
                    "reconnecting",
                    config_path,
                    config_healthy,
                );
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

/// Connect to evdev source with exponential backoff retry.
pub fn connect_evdev_with_retry(
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
                status::set_daemon_status_direct(
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
                status::set_daemon_status_direct(
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

/// Wait for a retry with cancellation support.
/// Returns true if we should retry, false if stop was requested.
pub fn wait_for_retry(stop: &AtomicBool, delay: Duration) -> bool {
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

/// Calculate next retry delay with exponential backoff (max 30s).
pub fn next_retry_delay(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::{input_poll_interval, ACTIVE_COMPLETION_POLL_INTERVAL, IDLE_STDIN_POLL_INTERVAL};
    use std::time::Duration;
    use wayexpand_core::CommandMetrics;

    #[test]
    fn stdin_idle_poll_keeps_control_requests_responsive() {
        assert_eq!(
            input_poll_interval(CommandMetrics::default()),
            IDLE_STDIN_POLL_INTERVAL
        );
        assert_eq!(IDLE_STDIN_POLL_INTERVAL, Duration::from_millis(20));
    }

    #[test]
    fn stdin_poll_accelerates_for_queued_command_work() {
        let queued = CommandMetrics {
            command_queue_depth: 1,
            ..CommandMetrics::default()
        };
        assert_eq!(input_poll_interval(queued), ACTIVE_COMPLETION_POLL_INTERVAL);
    }
}
