//! WayExpand Action Broker Service
//!
//! Standalone service that executes named actions with fine-grained per-action
//! permissions. It is shipped and integrated with daemon policy routing, but
//! remains operator-enabled because its action catalog is deployment-specific.
//!
//! Usage:
//!   wayexpand-action-broker --config ~/.config/wayexpand-broker.toml \
//!     --socket "$XDG_RUNTIME_DIR/wayexpand-broker.sock"

use action_broker::{
    config::{is_root_owner, is_user_or_root_owner},
    ipc::ConnectionCanceller,
    policy_hash, ActionError, ActionExecutor, AuditEvent, AuditHealth, AuditLogger, BrokerConfig,
    BrokerServer,
};
use anyhow::{anyhow, Result};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

const MAX_CONCURRENT_ACTIONS: usize = 16;
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

/// How long a response write may take once shutdown has begun, so a client
/// that stopped reading cannot hold up the broker for the full socket timeout.
const SHUTDOWN_WRITE_GRACE: Duration = Duration::from_secs(1);

/// SIGINT/SIGTERM listeners, installed before the broker starts serving so
/// a registration failure is a startup error rather than a later panic.
struct ShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

impl ShutdownSignals {
    fn install() -> Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())
                .map_err(|error| anyhow!("failed to install SIGINT handler: {error}"))?,
            terminate: signal(SignalKind::terminate())
                .map_err(|error| anyhow!("failed to install SIGTERM handler: {error}"))?,
        })
    }

    async fn wait(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }
}

/// Which blocking protocol step a connection is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IoPhase {
    ReadingRequest,
    WritingResponse,
}

/// Connections currently blocked on client I/O. On shutdown, clients that
/// have not finished sending a request are disconnected at once and response
/// writes get [`SHUTDOWN_WRITE_GRACE`]; connections whose action is running
/// are left alone so the action finishes within its own deadline and its
/// audit event is recorded.
#[derive(Default)]
struct ClientIo {
    shutting_down: bool,
    next_id: u64,
    active: std::collections::HashMap<u64, (IoPhase, ConnectionCanceller)>,
}

type ClientIoRegistry = Arc<std::sync::Mutex<ClientIo>>;

/// Removes a connection's entry when its blocking I/O step ends.
struct ClientIoGuard {
    registry: ClientIoRegistry,
    id: u64,
}

impl Drop for ClientIoGuard {
    fn drop(&mut self) {
        if let Ok(mut io) = self.registry.lock() {
            io.active.remove(&self.id);
        }
    }
}

fn apply_shutdown(phase: IoPhase, canceller: &ConnectionCanceller) {
    match phase {
        IoPhase::ReadingRequest => canceller.cancel(),
        IoPhase::WritingResponse => canceller.limit_writes(SHUTDOWN_WRITE_GRACE),
    }
}

/// Register a blocking I/O step; if shutdown already began, the step is
/// cancelled or bounded immediately.
fn track_client_io(
    registry: &ClientIoRegistry,
    phase: IoPhase,
    canceller: ConnectionCanceller,
) -> ClientIoGuard {
    let mut io = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if io.shutting_down {
        apply_shutdown(phase, &canceller);
    }
    let id = io.next_id;
    io.next_id = io.next_id.wrapping_add(1);
    io.active.insert(id, (phase, canceller));
    ClientIoGuard {
        registry: Arc::clone(registry),
        id,
    }
}

fn begin_client_io_shutdown(registry: &ClientIoRegistry) {
    let mut io = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    io.shutting_down = true;
    for (phase, canceller) in io.active.values() {
        apply_shutdown(*phase, canceller);
    }
}

struct BrokerOptions {
    config_file: PathBuf,
    socket_path: PathBuf,
    verbose: bool,
}

impl Default for BrokerOptions {
    fn default() -> Self {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| format!("/run/user/{}", rustix::process::getuid().as_raw()));
        Self {
            config_file: PathBuf::from(
                shellexpand::tilde("~/.config/wayexpand-broker.toml").into_owned(),
            ),
            socket_path: PathBuf::from(format!("{}/wayexpand-broker.sock", runtime_dir)),
            verbose: false,
        }
    }
}

fn parse_args() -> Result<BrokerOptions> {
    let mut options = BrokerOptions::default();
    let args: Vec<String> = std::env::args().collect();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                i += 1;
                if i >= args.len() {
                    return Err(anyhow!("--config requires an argument"));
                }
                options.config_file = PathBuf::from(&args[i]);
            }
            "--socket" => {
                i += 1;
                if i >= args.len() {
                    return Err(anyhow!("--socket requires an argument"));
                }
                options.socket_path = PathBuf::from(&args[i]);
            }
            "--verbose" | "-v" => {
                options.verbose = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            arg => {
                return Err(anyhow!("Unknown argument: {}", arg));
            }
        }
        i += 1;
    }

    Ok(options)
}

fn print_help() {
    const HELP: &str = r#"A secure command execution service with fine-grained per-action permissions.

USAGE:
    wayexpand-action-broker [OPTIONS]

OPTIONS:
    --config <PATH>
        Path to broker configuration file
        Default: ~/.config/wayexpand-broker.toml

    --socket <PATH>
        Unix socket path for IPC communication
        Default: $XDG_RUNTIME_DIR/wayexpand-broker.sock
        (fallback: /run/user/<current-uid>/wayexpand-broker.sock)

    --verbose, -v
        Enable verbose logging

    --help, -h
        Print this help message

EXAMPLE:
    wayexpand-action-broker \\
      --config ~/.config/wayexpand-broker.toml \\
      --socket "$XDG_RUNTIME_DIR/wayexpand-broker.sock" \\
      --verbose

CONFIGURATION:
    Create ~/.config/wayexpand-broker.toml with action definitions:

    [broker]
    # Defaults to true; set false only for intentional PATH-based resolution.
    require_absolute_paths = true
    strict_env = true
    # Optional JSONL execution audit sink (mode 0600, rotates at 16 MiB).
    # audit_path = "$XDG_STATE_HOME/wayexpand/action-audit.jsonl"
    # audit_required = false

    [actions."example"]
    program = "/usr/bin/example"
    args = []
    timeout_ms = 5000
    server_env = ["HOME"]
    enabled = true

For more information, see: https://github.com/cyberducttape/wayexpand
"#;
    println!(
        "WayExpand Action Broker Service v{}\n\n{}",
        env!("WAYEXPAND_BUILD_VERSION"),
        HELP
    );
}

fn load_config(path: &PathBuf) -> Result<(BrokerConfig, String)> {
    const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
    let resolved = fs::canonicalize(path)
        .map_err(|e| anyhow!("Failed to resolve config file {}: {}", path.display(), e))?;
    validate_config_ancestors(&resolved)?;
    let descriptor = rustix::fs::open(
        &resolved,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|e| anyhow!("Failed to open config file {}: {}", path.display(), e))?;
    let file = fs::File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|e| anyhow!("Failed to stat config file {}: {}", path.display(), e))?;
    if !metadata.file_type().is_file() {
        return Err(anyhow!(
            "config path is not a regular file: {}",
            path.display()
        ));
    }
    let current_uid = rustix::process::geteuid().as_raw();
    if !is_user_or_root_owner(metadata.uid(), current_uid) {
        return Err(anyhow!(
            "config file is not owned by the current user or root"
        ));
    }
    let mode = metadata.mode() & 0o777;
    if (metadata.uid() == current_uid && mode != 0o600)
        || (is_root_owner(metadata.uid()) && mode & 0o022 != 0)
    {
        return Err(anyhow!(
            "config file permissions are insecure (expected 0600 for user-owned files)"
        ));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(anyhow!("config file exceeds {} bytes", MAX_CONFIG_BYTES));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| anyhow!("Failed to read config file {}: {}", path.display(), e))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(anyhow!("config file exceeds {} bytes", MAX_CONFIG_BYTES));
    }
    let policy_hash = policy_hash(&bytes);
    let content =
        String::from_utf8(bytes).map_err(|e| anyhow!("config file is not valid UTF-8: {}", e))?;

    let mut config = BrokerConfig::from_toml(&content)
        .map_err(|e| anyhow!("Failed to parse config file: {}", e))?;

    config
        .validate_and_canonicalize()
        .map_err(|e| anyhow!("Config validation failed: {}", e))?;

    Ok((config, policy_hash))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn audit_exit_status(response: &action_broker::ActionResponse) -> Option<i32> {
    match response {
        action_broker::ActionResponse::Success(output) => Some(output.exit_code),
        action_broker::ActionResponse::Error(ActionError::ExitFailure { exit_code, .. }) => {
            Some(*exit_code)
        }
        _ => None,
    }
}

fn audit_output_size(response: &action_broker::ActionResponse) -> usize {
    match response {
        action_broker::ActionResponse::Success(output) => output.stdout.len() + output.stderr.len(),
        action_broker::ActionResponse::Error(ActionError::ExitFailure { stderr, .. }) => {
            stderr.len()
        }
        _ => 0,
    }
}

fn broker_health_path(socket_path: &Path) -> PathBuf {
    socket_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("wayexpand-broker-health.json")
}

fn write_broker_health(
    path: &Path,
    audit_logger: Option<&AuditLogger>,
    audit_required: bool,
    running: bool,
) -> std::io::Result<()> {
    let health = audit_logger
        .map(AuditLogger::health)
        .unwrap_or(AuditHealth {
            dropped_events: 0,
            write_failures: 0,
        });
    let status = serde_json::json!({
        "pid": std::process::id(),
        "running": running,
        "audit_enabled": audit_logger.is_some(),
        "audit_required": audit_logger.is_some() && audit_required,
        "audit_queue_dropped_total": health.dropped_events,
        "audit_write_failures_total": health.write_failures,
        "audit_healthy": audit_logger.is_none() || health.healthy(),
    });
    let temporary = path.with_file_name(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("wayexpand-broker-health.json"),
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, &status)
        .map_err(|error| std::io::Error::other(format!("serialize broker health: {error}")))?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn validate_config_ancestors(path: &Path) -> Result<()> {
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = path
        .parent()
        .ok_or_else(|| anyhow!("config file has no parent directory"))?;
    loop {
        let metadata = fs::metadata(current).map_err(|e| {
            anyhow!(
                "Failed to inspect config ancestor {}: {}",
                current.display(),
                e
            )
        })?;
        if !metadata.is_dir() {
            return Err(anyhow!("config ancestor is not a directory"));
        }
        if !is_user_or_root_owner(metadata.uid(), current_uid) {
            return Err(anyhow!(
                "config ancestor '{}' is not owned by the current user or root (owner uid {}, current uid {})",
                current.display(),
                metadata.uid(),
                current_uid
            ));
        }
        if metadata.mode() & 0o022 != 0 {
            return Err(anyhow!(
                "config ancestor is writable by group or other users"
            ));
        }
        if current == Path::new("/") {
            break;
        }
        current = current
            .parent()
            .ok_or_else(|| anyhow!("config ancestor traversal failed"))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging. Do not set RUST_LOG here: the Tokio runtime's
    // worker threads already exist, and mutating the environment while
    // other threads may read it is unsound.
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

    // Parse command-line arguments
    let options = parse_args()?;
    let mut shutdown_signals = ShutdownSignals::install()?;

    info!(
        config_file = %options.config_file.display(),
        socket_path = %options.socket_path.display(),
        verbose = options.verbose,
        "starting wayexpand action broker"
    );

    // Load configuration
    let (config, policy_hash) = load_config(&options.config_file)?;
    info!(
        action_count = config.actions.len(),
        require_absolute_paths = config.require_absolute_paths,
        "loaded broker configuration"
    );

    // Create executor with loaded config
    let executor = Arc::new(
        ActionExecutor::new(&config).map_err(|e| anyhow!("Failed to create executor: {}", e))?,
    );
    let audit_required = config.audit_required;
    if audit_required && config.audit_path.is_none() {
        return Err(anyhow!("audit_required requires audit_path"));
    }
    let audit_logger = match config.audit_path.as_deref() {
        Some(path) => {
            let logger = AuditLogger::new(Path::new(path), policy_hash.clone())
                .map_err(|e| anyhow!("Failed to open audit log {}: {}", path, e))?;
            info!(audit_path = %path, policy_hash = %logger.policy_hash(), "execution audit enabled");
            Some(Arc::new(logger))
        }
        None => None,
    };
    let health_path = broker_health_path(&options.socket_path);

    // Create server socket
    let server = Arc::new(
        BrokerServer::bind(&options.socket_path)
            .map_err(|e| anyhow!("Failed to bind broker socket: {}", e))?,
    );
    if let Err(error) =
        write_broker_health(&health_path, audit_logger.as_deref(), audit_required, true)
    {
        warn!(error = %error, path = %health_path.display(), "failed to publish broker health");
    }
    let action_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_ACTIONS));
    let connection_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let verbose = options.verbose;
    let request_counter = Arc::new(AtomicU64::new(1));

    info!(
        socket_path = %options.socket_path.display(),
        "broker listening for connections"
    );

    // Main service loop - accept connections and handle requests
    let mut connection_tasks = tokio::task::JoinSet::new();
    let client_io: ClientIoRegistry = Arc::default();
    let mut shutdown = Box::pin(shutdown_signals.wait());
    loop {
        // Finished tasks still hold their captured Arcs until their join
        // handle is reaped. Keep the set bounded over a long-running broker
        // rather than retaining one handle per historical connection.
        while let Some(result) = connection_tasks.try_join_next() {
            if let Err(error) = result {
                error!(error = %error, "broker connection task failed");
            }
        }
        let accept_server = Arc::clone(&server);
        let accepted = tokio::select! {
            _ = &mut shutdown => {
                info!("shutdown signal received; flushing audit events");
                begin_client_io_shutdown(&client_io);
                let wake_path = server.socket_path().to_owned();
                let _ = tokio::task::spawn_blocking(move || {
                    std::os::unix::net::UnixStream::connect(wake_path)
                })
                .await;
                break;
            }
            accepted = tokio::task::spawn_blocking(move || accept_server.accept()) => accepted,
        };
        let conn = match accepted {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                warn!("failed to accept connection: {}", e);
                continue;
            }
            Err(e) => {
                warn!("accept task failed: {}", e);
                continue;
            }
        };
        let connection_permit = match connection_slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                warn!(
                    maximum = MAX_CONCURRENT_CONNECTIONS,
                    "broker connection limit reached; rejecting client"
                );
                continue;
            }
        };
        info!("accepted broker client connection");
        let executor = Arc::clone(&executor);
        let audit_logger = audit_logger.clone();
        let health_path = health_path.clone();
        let request_counter = Arc::clone(&request_counter);
        let action_slots = Arc::clone(&action_slots);
        let client_io = Arc::clone(&client_io);
        connection_tasks.spawn(async move {
            let _connection_permit = connection_permit;
            let track = |phase, conn: &action_broker::ipc::ServerConnection| {
                conn.canceller()
                    .map(|canceller| track_client_io(&client_io, phase, canceller))
            };
            let read_guard = match track(IoPhase::ReadingRequest, &conn) {
                Ok(guard) => guard,
                Err(e) => {
                    error!("failed to prepare connection: {}", e);
                    return;
                }
            };
            let read = tokio::task::spawn_blocking(move || {
                let mut conn = conn;
                let request = conn.read_request()?;
                Ok::<_, action_broker::ipc::IpcError>((conn, request))
            })
            .await;
            drop(read_guard);
            let (mut conn, request) = match read
            {
                Ok(Ok(value)) => value,
                Ok(Err(e)) => {
                    error!("failed to read request: {}", e);
                    return;
                }
                Err(e) => {
                    error!("read task failed: {}", e);
                    return;
                }
            };
            let peer = conn.peer_identity().clone();
            let request_id = format!(
                "{}-{}",
                std::process::id(),
                request_counter.fetch_add(1, Ordering::Relaxed)
            );
            if verbose {
                info!(action_id = %request.action_id, timeout_ms = request.timeout_ms, "received action request");
            }
            let action_id = request.action_id.clone();
            let permit = match action_slots.try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    let response = action_broker::ActionResponse::Error(
                        action_broker::ActionError::ActionBlocked {
                            action_id: action_id.clone(),
                            reason: format!(
                                "broker concurrency limit reached (maximum {})",
                                MAX_CONCURRENT_ACTIONS
                            ),
                        },
                    );
                    let _write_guard = track(IoPhase::WritingResponse, &conn).ok();
                    match tokio::task::spawn_blocking(move || conn.write_response(&response)).await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            error!(action_id = %action_id, error = %e, "failed to send busy response")
                        }
                        Err(e) => {
                            error!(action_id = %action_id, error = %e, "busy response task failed")
                        }
                    }
                    return;
                }
            };
            let _permit = permit;
            let started_at = unix_time_ms();
            let started = Instant::now();
            let mut action_response = match executor.execute(request).await {
                Ok(output) => output,
                Err(error) => action_broker::ActionResponse::Error(error),
            };
            if let Some(logger) = &audit_logger {
                let finished_at = unix_time_ms();
                let event = AuditEvent {
                    timestamp: started_at,
                    request_id: &request_id,
                    action_id: &action_id,
                    caller_pid: peer.pid,
                    caller_executable: peer.executable.as_deref(),
                    policy_hash: logger.policy_hash(),
                    start: started_at,
                    finish: finished_at,
                    duration_ms: started.elapsed().as_millis() as u64,
                    exit_status: audit_exit_status(&action_response),
                    timed_out: matches!(
                        action_response,
                        action_broker::ActionResponse::Error(ActionError::Timeout { .. })
                    ),
                    output_size: audit_output_size(&action_response),
                };
                let audit_result = if audit_required {
                    // Waits for an fsync (up to the writer's confirmation
                    // timeout); keep it off the async worker like the other
                    // blocking broker I/O. The runtime is multi-threaded.
                    tokio::task::block_in_place(|| logger.record_required(&event))
                } else {
                    logger.record(&event)
                };
                if let Err(error) = audit_result {
                    error!(request_id = %request_id, error = %error, "failed to write action audit event");
                    if audit_required {
                        action_response = action_broker::ActionResponse::Error(
                            ActionError::Internal {
                                reason: format!("mandatory action audit failed: {error}"),
                            },
                        );
                    }
                }
                if let Err(error) = write_broker_health(&health_path, Some(logger), audit_required, true) {
                    warn!(error = %error, path = %health_path.display(), "failed to publish broker health");
                }
            }
            let success = action_response.is_success();
            let _write_guard = track(IoPhase::WritingResponse, &conn).ok();
            match tokio::task::spawn_blocking(move || conn.write_response(&action_response)).await {
                Ok(Ok(())) if verbose => info!(action_id = %action_id, success, "sent response"),
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!(action_id = %action_id, error = %e, "failed to send response"),
                Err(e) => error!(action_id = %action_id, error = %e, "write task failed"),
            }
        });
    }

    // Connection tasks may still be finishing an action or recording its
    // audit event when the accept loop receives SIGTERM. Await them before
    // dropping the last AuditLogger Arc; otherwise Tokio runtime teardown can
    // discard accepted audit events.
    while let Some(result) = connection_tasks.join_next().await {
        if let Err(error) = result {
            error!(error = %error, "broker connection task failed during shutdown");
        }
    }
    if let Err(error) =
        write_broker_health(&health_path, audit_logger.as_deref(), audit_required, false)
    {
        warn!(error = %error, path = %health_path.display(), "failed to publish broker health");
    }
    if let Some(logger) = audit_logger.as_deref() {
        let health = logger.health();
        info!(
            audit_queue_dropped_total = health.dropped_events,
            audit_write_failures_total = health.write_failures,
            audit_healthy = health.healthy(),
            "action audit health"
        );
    }
    drop(server);
    drop(audit_logger);
    info!("action broker stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn action_slots_reject_work_after_the_fixed_limit() {
        let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_ACTIONS));
        let mut permits = Vec::with_capacity(MAX_CONCURRENT_ACTIONS);
        for _ in 0..MAX_CONCURRENT_ACTIONS {
            permits.push(slots.clone().try_acquire_owned().unwrap());
        }
        assert!(slots.try_acquire().is_err());
        drop(permits);
        assert!(slots.try_acquire().is_ok());
    }

    #[tokio::test]
    async fn connection_slots_reject_clients_before_request_handling() {
        let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
        let mut permits = Vec::with_capacity(MAX_CONCURRENT_CONNECTIONS);
        for _ in 0..MAX_CONCURRENT_CONNECTIONS {
            permits.push(slots.clone().try_acquire_owned().unwrap());
        }
        assert!(slots.try_acquire().is_err());
        drop(permits);
        assert!(slots.try_acquire().is_ok());
    }
}
