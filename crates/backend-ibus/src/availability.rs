//! Installation and registration probes for the native IBus component.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Return whether the installed IBus component can be discovered by setup.
/// This is intentionally an installation/provisioning probe, not an
/// end-to-end typing guarantee.
pub fn engine_available() -> bool {
    if !executable_in_path("ibus") || !executable_in_path("wayexpand-ibus") {
        return false;
    }

    if component_file_present_in(&ibus_component_directories()) {
        return true;
    }

    ibus_registry_contains_engine()
}

fn ibus_registry_contains_engine() -> bool {
    let Ok(mut child) = Command::new("ibus")
        .args(["list-engine"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map(|output| {
                        output.status.success()
                            && String::from_utf8_lossy(&output.stdout)
                                .lines()
                                .any(|line| line.contains("wayexpand"))
                    })
                    .unwrap_or(false);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn executable_in_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| {
        let candidate = directory.join(name);
        fs::metadata(candidate)
            .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

pub(super) fn component_file_present_in(directories: &[PathBuf]) -> bool {
    directories
        .iter()
        .map(|directory| directory.join("wayexpand.xml"))
        .any(|path| path.is_file())
}

fn ibus_component_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        directories.push(PathBuf::from(data_home).join("ibus/component"));
    } else if let Some(home) = env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".local/share/ibus/component"));
    }
    directories.push(PathBuf::from("/usr/local/share/ibus/component"));
    directories.push(PathBuf::from("/usr/share/ibus/component"));
    directories
}
