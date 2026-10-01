use std::env;
use wayexpand_backend_ibus::run_service;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let config = env::args_os().nth(1).map(Into::into);
    run_service(config)?;
    Ok(())
}
