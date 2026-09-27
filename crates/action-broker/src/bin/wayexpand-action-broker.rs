//! WayExpand Action Broker Service
//!
//! Standalone service that executes actions with fine-grained per-action permissions.
//! Phase 1 stub: accepts connections and executes commands according to policy.
//!
//! Usage:
//!   wayexpand-action-broker --config ~/.config/wayexpand-broker.toml \
//!     --socket /run/user/1000/wayexpand-broker.sock

use action_broker::{ActionExecutor, BrokerConfig, BrokerServer};
use anyhow::{anyhow, Result};
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{error, info, warn};

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
    eprintln!(
        r#"WayExpand Action Broker Service v1.3.0

A secure command execution service with fine-grained per-action permissions.

USAGE:
    wayexpand-action-broker [OPTIONS]

OPTIONS:
    --config <PATH>
        Path to broker configuration file
        Default: ~/.config/wayexpand-broker.toml

    --socket <PATH>
        Unix socket path for IPC communication
        Default: /run/user/1000/wayexpand-broker.sock

    --verbose, -v
        Enable verbose logging

    --help, -h
        Print this help message

EXAMPLE:
    wayexpand-action-broker \\
      --config ~/.config/wayexpand-broker.toml \\
      --socket /run/user/1000/wayexpand-broker.sock \\
      --verbose

CONFIGURATION:
    Create ~/.config/wayexpand-broker.toml with action definitions:

    [broker]
    require_absolute_paths = true
    strict_env = true

    [actions."example"]
    program = "/usr/bin/example"
    args_prefix = []
    timeout_ms = 5000
    pass_env = ["HOME"]
    enabled = true

For more information, see: https://github.com/cyberducttape/wayexpand
"#
    );
}

fn load_config(path: &PathBuf) -> Result<BrokerConfig> {
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
    if metadata.uid() != current_uid && metadata.uid() != 0 {
        return Err(anyhow!(
            "config file is not owned by the current user or root"
        ));
    }
    let mode = metadata.mode() & 0o777;
    if (metadata.uid() == current_uid && mode != 0o600)
        || (metadata.uid() == 0 && mode & 0o022 != 0)
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
    let content =
        String::from_utf8(bytes).map_err(|e| anyhow!("config file is not valid UTF-8: {}", e))?;

    let config = BrokerConfig::from_toml(&content)
        .map_err(|e| anyhow!("Failed to parse config file: {}", e))?;

    config
        .validate()
        .map_err(|e| anyhow!("Config validation failed: {}", e))?;

    Ok(config)
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
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            return Err(anyhow!(
                "config ancestor is not owned by the current user or root"
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
    // Initialize logging
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::init();

    // Parse command-line arguments
    let options = parse_args()?;

    info!(
        config_file = %options.config_file.display(),
        socket_path = %options.socket_path.display(),
        verbose = options.verbose,
        "starting wayexpand action broker"
    );

    // Load configuration
    let config = load_config(&options.config_file)?;
    info!(
        action_count = config.actions.len(),
        require_absolute_paths = config.require_absolute_paths,
        "loaded broker configuration"
    );

    // Create executor with loaded config
    let executor = Arc::new(
        ActionExecutor::new(&config).map_err(|e| anyhow!("Failed to create executor: {}", e))?,
    );

    // Create server socket
    let server = Arc::new(
        BrokerServer::bind(&options.socket_path)
            .map_err(|e| anyhow!("Failed to bind broker socket: {}", e))?,
    );
    let verbose = options.verbose;

    info!(
        socket_path = %options.socket_path.display(),
        "broker listening for connections"
    );

    // Main service loop - accept connections and handle requests
    loop {
        let accept_server = Arc::clone(&server);
        let accepted = tokio::task::spawn_blocking(move || accept_server.accept()).await;
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
        info!("accepted broker client connection");
        let executor = Arc::clone(&executor);
        tokio::spawn(async move {
            let (mut conn, request) = match tokio::task::spawn_blocking(move || {
                let mut conn = conn;
                let request = conn.read_request()?;
                Ok::<_, action_broker::ipc::IpcError>((conn, request))
            })
            .await
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
            if verbose {
                info!(action_id = %request.action_id, timeout_ms = request.timeout_ms, "received action request");
            }
            let action_id = request.action_id.clone();
            let action_response = match executor.execute(request).await {
                Ok(output) => output,
                Err(error) => action_broker::ActionResponse::Error(error),
            };
            let success = action_response.is_success();
            match tokio::task::spawn_blocking(move || conn.write_response(&action_response)).await {
                Ok(Ok(())) if verbose => info!(action_id = %action_id, success, "sent response"),
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!(action_id = %action_id, error = %e, "failed to send response"),
                Err(e) => error!(action_id = %action_id, error = %e, "write task failed"),
            }
        });
    }
}
