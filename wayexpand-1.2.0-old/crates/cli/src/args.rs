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
