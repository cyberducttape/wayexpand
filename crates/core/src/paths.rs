use std::{env, path::PathBuf};

/// Resolve the user configuration file without depending on the daemon's
/// working directory. `WAYEXPAND_CONFIG` is intended for service managers and
/// tests; normal users get the XDG config location.
pub fn default_config_path() -> PathBuf {
    if let Some(path) = env::var_os("WAYEXPAND_CONFIG") {
        return PathBuf::from(path);
    }
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        let config_home = PathBuf::from(config_home);
        if config_home.is_absolute() {
            return config_home.join("wayexpand/expansions.toml");
        }
    }
    if let Some(user_home) = env::var_os("HOME") {
        return PathBuf::from(user_home).join(".config/wayexpand/expansions.toml");
    }
    PathBuf::from("expansions.toml")
}
