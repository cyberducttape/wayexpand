use std::env;
use wayexpand_backend_ibus::run_service;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = env::args_os().nth(1).map(Into::into);
    run_service(config)?;
    Ok(())
}
