//! Input source connection and reconnection logic.
//!
//! Handles:
//! - Input-method-v2 backend with libei key pass-through
//! - Evdev source connection with device discovery
//! - Exponential backoff retry logic
//! - Adaptive polling intervals based on command queue depth

use anyhow::Result;
use std::{
    path::Path,
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};
use wayexpand_backend_evdev::EvdevSource;
use wayexpand_backend_input_method::{InputMethodError, InputMethodSource};
use wayexpand_core::{CommandMetrics, OrganizationPolicy, TextInjector};

/// Low-latency polling while command/hotkey work is queued or running.
const ACTIVE_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Idle maintenance cadence for reload/pause/stop checks.
const IDLE_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(250);

pub use crate::output_loop::connect_output_backend;
use crate::{control, status};

/// Determine polling interval based on async work queue depth.
pub fn input_poll_interval(metrics: CommandMetrics) -> Duration {
    if metrics.command_queue_depth > 0 || metrics.command_in_flight > 0 {
        ACTIVE_COMPLETION_POLL_INTERVAL
    } else {
        IDLE_MAINTENANCE_INTERVAL
    }
}

/// Connect to input-method-v2 with optional libei key pass-through.
pub fn connect_input_method_session(
    control: &control::ControlServer,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path: Option<&Path>,
    policy: &OrganizationPolicy,
) -> Result<InputMethodSource, InputMethodError> {
    let mut source = InputMethodSource::connect()?;

    if !policy.backend_allowed("libei") {
        let violation = format!(
            "backend 'libei' is not in allowed list: {:?}",
            policy.allowed_backends
        );
        crate::policy::log_violation(policy, &violation);
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
            let backend_mode = key_injector.status_detail();
            source = source.with_key_pass_through(key_injector);
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
        }
        Err(error) if error.retryable => {
            warn!(
                %error,
                "libei unavailable at startup; unsupported keys will not pass through \
                (connection will be retried asynchronously)"
            );
            status::set_daemon_status_direct(
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
