use anyhow::{bail, Result};

/// Pulls the first `--json` flag out of `args`, wherever it appears.
pub fn take_json_flag(args: &mut Vec<String>) -> bool {
    if let Some(position) = args.iter().position(|arg| arg == "--json") {
        args.remove(position);
        true
    } else {
        false
    }
}

/// Removes an option in either `--name=value` or `--name value` form.
pub fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let prefix = format!("{name}=");
    let Some(index) = args
        .iter()
        .position(|arg| arg == name || arg.starts_with(&prefix))
    else {
        return Ok(None);
    };
    let value = if let Some(value) = args[index].strip_prefix(&prefix) {
        value.to_owned()
    } else if index + 1 < args.len() {
        let value = args[index + 1].clone();
        args.remove(index + 1);
        value
    } else {
        bail!("{name} requires a value");
    };
    args.remove(index);
    if value.is_empty() {
        bail!("{name} requires a non-empty value");
    }
    Ok(Some(value))
}

/// Removes a boolean flag wherever it appears.
pub fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

/// Turn the optional `[config]` argument into a path. A leftover argument
/// that looks like an option is an unknown option, not a file name: without
/// this, `wayexpand stats --bogus` reported a missing configuration file
/// called `--bogus`. A file whose name starts with `-` is still reachable
/// as `./-name`.
pub(crate) fn config_path_arg(value: impl Into<String>) -> Result<std::path::PathBuf> {
    let value = value.into();
    if value.starts_with('-') {
        return Err(crate::usage_error(format!("unknown option {value:?}")));
    }
    Ok(std::path::PathBuf::from(value))
}
