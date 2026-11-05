//! Configuration inspection commands that do not alter the active file.

use super::Args;
use crate::*;

fn path_or_default(value: Option<String>) -> PathBuf {
    value.map(PathBuf::from).unwrap_or_else(default_config_path)
}

fn collect_changes(
    left: &serde_json::Value,
    right: &serde_json::Value,
    path: &str,
    out: &mut Vec<String>,
) {
    match (left, right) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            let keys = a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>();
            for key in keys {
                let child = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => collect_changes(x, y, &child, out),
                    _ => out.push(child),
                }
            }
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            for index in 0..a.len().max(b.len()) {
                let child = format!("{path}/{index}");
                match (a.get(index), b.get(index)) {
                    (Some(x), Some(y)) => collect_changes(x, y, &child, out),
                    _ => out.push(child),
                }
            }
        }
        _ if left != right => out.push(path.to_owned()),
        _ => {}
    }
}

pub(crate) fn config_command(mut args: Args) -> Result<()> {
    let operation = args
        .next()
        .ok_or_else(|| usage_error("usage: wayexpand config show|validate|diff ..."))?;
    match operation.as_str() {
        "show" => {
            let mut json = false;
            let mut path = None;
            for value in args {
                if value == "--json" {
                    json = true;
                } else if value.starts_with('-') || path.is_some() {
                    usage_bail!("usage: wayexpand config show [--json] [config]");
                } else {
                    path = Some(value);
                }
            }
            let path = path_or_default(path);
            let config = Config::load(&path).map_err(|error| config_load_error(&path, error))?;
            let value = serde_json::json!({
                "schema_version":1,
                "config":config,
                "note":"This command displays local configuration, including replacement text. Review output before sharing it."
            });
            if json {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&value["config"])?);
            }
        }
        "validate" => {
            let mut json = false;
            let mut path = None;
            for value in args {
                if value == "--json" {
                    json = true;
                } else if value.starts_with('-') || path.is_some() {
                    usage_bail!("usage: wayexpand config validate [--json] [config]");
                } else {
                    path = Some(value);
                }
            }
            let path = path_or_default(path);
            let mut config =
                Config::load(&path).map_err(|error| config_load_error(&path, error))?;
            Config::validate_library_files(&path)
                .map_err(|error| config_error(error.safe_summary()))?;
            let policy = crate::doctor::policy::load_policy()?;
            config
                .apply_administrator_policy(&policy)
                .map_err(|error| config_error(error.safe_summary()))?;
            let value = serde_json::json!({"valid":true,"expansions":config.expansion.len(),"enabled":config.expansion.iter().filter(|e|e.enabled).count(),"dynamic_commands_validated":config.expansion.iter().filter(|e|e.command.is_some()).count()+config.hotkey.len(),"dynamic_commands_executed":false});
            if json {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("configuration valid: {} expansion(s); dynamic declarations validated, none executed", value["expansions"]);
            }
        }
        "diff" => {
            let mut json = false;
            let mut paths = Vec::new();
            for value in args {
                if value == "--json" {
                    json = true;
                } else if value.starts_with('-') {
                    usage_bail!("usage: wayexpand config diff [--json] OLD NEW");
                } else {
                    paths.push(PathBuf::from(value));
                }
            }
            if paths.len() != 2 {
                usage_bail!("usage: wayexpand config diff [--json] OLD NEW");
            }
            let old =
                Config::load(&paths[0]).map_err(|error| config_load_error(&paths[0], error))?;
            let new =
                Config::load(&paths[1]).map_err(|error| config_load_error(&paths[1], error))?;
            let old = serde_json::to_value(old)?;
            let new = serde_json::to_value(new)?;
            let mut changed = Vec::new();
            collect_changes(&old, &new, "", &mut changed);
            let result = serde_json::json!({"changed_fields":changed,"change_count":changed.len(),"values_included":false});
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if changed.is_empty() {
                println!("configurations have no differences");
            } else {
                for path in changed {
                    println!("changed {path}");
                }
            }
        }
        _ => usage_bail!("usage: wayexpand config show|validate|diff ..."),
    }
    Ok(())
}
