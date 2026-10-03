//! Command-line and environment argument parsing.

use crate::*;

pub(crate) type DaemonArgs = (PathBuf, Option<String>, Option<String>, bool, bool);

pub(crate) fn parse_args() -> Result<DaemonArgs> {
    let env_path = env::var_os("WAYEXPAND_CONFIG").map(PathBuf::from);
    let mut path = env_path.clone();
    let mut explicit_path = env_path.is_some();
    let mut backend = env::var("WAYEXPAND_BACKEND").ok();
    let mut source = env::var("WAYEXPAND_SOURCE").ok();
    let mut allow_evdev_sensitive_fields = false;
    for argument in env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--backend=") {
            backend = Some(value.to_string());
        } else if let Some(value) = argument.strip_prefix("--source=") {
            source = Some(value.to_string());
        } else if argument == "--allow-evdev-sensitive-fields" {
            allow_evdev_sensitive_fields = true;
        } else if matches!(argument.as_str(), "--help" | "-h") {
            println!("wayexpand-daemon {}\nusage: wayexpand-daemon [--source=stdin|input-method|evdev] [--backend=none|wlroots|libei] [--allow-evdev-sensitive-fields] [config]", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        } else if matches!(argument.as_str(), "--version" | "-V") {
            println!("wayexpand-daemon {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        } else if argument.starts_with('-') {
            anyhow::bail!("unknown option {argument:?}; try --help");
        } else if path.is_some() {
            anyhow::bail!("multiple configuration paths supplied");
        } else {
            path = Some(PathBuf::from(argument));
            explicit_path = true;
        }
    }
    Ok((
        path.unwrap_or_else(default_config_path),
        source,
        backend,
        allow_evdev_sensitive_fields,
        !explicit_path,
    ))
}
