use std::env;
use wayexpand_backend_ibus::run_service;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = env::args_os().nth(1).map(Into::into);
    run_service(config)?;
    Ok(())
}
