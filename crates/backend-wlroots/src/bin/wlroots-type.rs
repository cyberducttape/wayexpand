use anyhow::{Context, Result};
use std::env;
use wayexpand_backend_wlroots::WlrootsInjector;
use wayexpand_core::TextInjector;

fn main() -> Result<()> {
    let text = env::args().skip(1).collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        anyhow::bail!("usage: wlroots-type <text>");
    }
    let mut injector =
        WlrootsInjector::connect().context("connecting to the wlroots virtual keyboard")?;
    injector.insert(&text).context("injecting text")?;
    Ok(())
}
