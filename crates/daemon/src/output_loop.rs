//! Output backend connection and reconnection logic.
//!
//! Handles:
//! - Wlroots and libei output backend selection
//! - Portal token persistence for libei sandboxed access
//! - Exponential backoff retry logic
//! - Transient vs permanent error handling

use anyhow::Result;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};
use wayexpand_backend_libei::{LibeiInjector, LibeiOptions};
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::TextInjector;

use crate::{control, input_loop::wait_for_retry, status};

/// Error connecting to an output backend.
#[derive(Debug)]
pub struct OutputConnectError {
    pub message: String,
    pub retryable: bool,
}

impl std::fmt::Display for OutputConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OutputConnectError {}

/// Connect to the specified output backend.
pub fn connect_output_backend(
    name: &str,
    persist_portal_token: bool,
    portal_token_path_arg: Option<&Path>,
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
            portal_token_path: portal_token_path_arg.map(Path::to_path_buf),
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

/// Connect to an output backend with exponential backoff retry.
pub fn connect_output_with_retry(
    control: &control::ControlServer,
    source: &str,
    backend: &str,
    config_path: &Path,
    config_healthy: bool,
    persist_portal_token: bool,
    portal_token_path_arg: Option<&Path>,
) -> Result<Option<Box<dyn TextInjector>>> {
    let mut retry_delay = Duration::from_millis(250);
    loop {
        match connect_output_backend(backend, persist_portal_token, portal_token_path_arg) {
            Ok(injector) => {
                status::set_daemon_status_with_mode(
                    control,
                    source,
                    backend,
                    "connected",
                    config_path,
                    config_healthy,
                    if injector.status_detail().is_empty() {
                        "unknown"
                    } else {
                        injector.status_detail()
                    },
                    wayexpand_core::CommandMetrics::default(),
                );
                info!(backend, "output backend reconnected");
                return Ok(Some(injector));
            }
            Err(error) if error.retryable => {
                warn!(%error, backend, ?retry_delay, "output backend unavailable; retrying");
                status::set_daemon_status_direct(
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

/// Calculate next retry delay with exponential backoff (max 30s).
fn next_retry_delay(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(Duration::from_secs(30))
}
