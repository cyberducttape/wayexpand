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
use std::path::PathBuf;
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
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("Failed to read config file {}: {}", path.display(), e))?;

    let config = BrokerConfig::from_toml(&content)
        .map_err(|e| anyhow!("Failed to parse config file: {}", e))?;

    config
        .validate()
        .map_err(|e| anyhow!("Config validation failed: {}", e))?;

    Ok(config)
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
    let executor =
        ActionExecutor::new(&config).map_err(|e| anyhow!("Failed to create executor: {}", e))?;

    // Create server socket
    let server = BrokerServer::bind(&options.socket_path)
        .map_err(|e| anyhow!("Failed to bind broker socket: {}", e))?;

    info!(
        socket_path = %options.socket_path.display(),
        "broker listening for connections"
    );

    // Main service loop - accept connections and handle requests
    loop {
        match server.accept() {
            Ok(mut conn) => {
                info!("accepted broker client connection");

                // Read request from client
                match conn.read_request() {
                    Ok(request) => {
                        if options.verbose {
                            info!(
                                action_id = %request.action_id,
                                timeout_ms = request.timeout_ms,
                                "received action request"
                            );
                        }

                        // Execute action
                        let action_response = match executor.execute(request.clone()).await {
                            Ok(output) => output,
                            Err(error) => action_broker::ActionResponse::Error(error),
                        };

                        // Send response back to client
                        if let Err(e) = conn.write_response(&action_response) {
                            error!(
                                action_id = %request.action_id,
                                error = %e,
                                "failed to send response to client"
                            );
                        } else if options.verbose {
                            info!(
                                action_id = %request.action_id,
                                success = action_response.is_success(),
                                "sent response to client"
                            );
                        }
                    }
                    Err(e) => {
                        error!("failed to read request from client: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("failed to accept connection: {}", e);
                // Continue accepting connections instead of exiting
            }
        }
    }
}
