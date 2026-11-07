//! Installation and registration probes for the native IBus component.

use std::{
    env, fs,
    io::Read,
    os::fd::AsRawFd,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

/// The engine name declared in `desktop/wayexpand-ibus.xml`.
pub(crate) const ENGINE_NAME: &str = "wayexpand";
const MAX_IBUS_REGISTRY_OUTPUT_BYTES: usize = 64 * 1024;
const IBUS_REGISTRY_READS_PER_DRAIN: usize = 64;

#[derive(Default)]
struct RegistryDrain {
    truncated: bool,
    eof: bool,
}

fn set_nonblocking<R: AsRawFd>(stream: &R) -> Result<(), std::io::Error> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn drain_registry_output(
    stdout: &mut impl Read,
    output: &mut Vec<u8>,
) -> Result<RegistryDrain, std::io::Error> {
    let mut buffer = [0_u8; 8192];
    let mut result = RegistryDrain::default();
    for _ in 0..IBUS_REGISTRY_READS_PER_DRAIN {
        match stdout.read(&mut buffer) {
            Ok(0) => {
                result.eof = true;
                return Ok(result);
            }
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                if output.len() > MAX_IBUS_REGISTRY_OUTPUT_BYTES {
                    result.truncated = true;
                    return Ok(result);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(result);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

/// Return whether the running IBus daemon has registered the WayExpand engine.
///
/// A component file alone is not enough: ibus-daemon reads only its built-in
/// component directory (or `IBUS_COMPONENT_PATH`), never a per-user data
/// directory, so a manifest elsewhere can never be selected. This asks the
/// live registry instead. It is a registration probe, not an end-to-end
/// typing guarantee.
pub fn engine_available() -> bool {
    runtime_executables_present() && ibus_registry_contains_engine()
}

/// Return whether the WayExpand component is installed where ibus-daemon
/// loads components, so restarting IBus can register it.
///
/// Setup uses this before `ibus restart`; once IBus is running,
/// [`engine_available`] is the authoritative check.
pub fn engine_installed() -> bool {
    runtime_executables_present() && component_file_present_in(&ibus_component_directories())
}

/// Return whether IBus reports the WayExpand engine as the global engine.
///
/// `ibus engine <name>` cannot confirm a switch by its exit status: for an
/// engine whose layout is `default` (as ours is, so the user's layout is kept)
/// the CLI switches the engine and then still exits 1 unless
/// `use-system-keyboard-layout` is set. Reading the global engine back is the
/// reliable check.
pub fn global_engine_selected() -> bool {
    executable_in_path("ibus")
        && registry_command_contains_engine("ibus", &["engine"], Duration::from_secs(5))
}

fn runtime_executables_present() -> bool {
    executable_in_path("ibus") && executable_in_path("wayexpand-ibus")
}

pub(crate) fn ibus_registry_contains_engine() -> bool {
    registry_command_contains_engine(
        "ibus",
        &["list-engine", "--name-only"],
        Duration::from_secs(5),
    )
}

fn registry_command_contains_engine(program: &str, args: &[&str], timeout: Duration) -> bool {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_group(&mut command);
    let Ok(child) = command.spawn() else {
        return false;
    };
    let mut supervisor = ChildSupervisor::new(child);
    let Some(mut stdout) = supervisor.take_stdout() else {
        return false;
    };
    if set_nonblocking(&stdout).is_err() {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return false;
    }
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    loop {
        let drain = match drain_registry_output(&mut stdout, &mut output) {
            Ok(drain) => drain,
            Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return false;
            }
        };
        if drain.truncated {
            supervisor.kill_group();
            let _ = supervisor.reap();
            return false;
        }
        match supervisor.has_exited() {
            Ok(true) => {
                // A child can exit while a descendant still holds stdout.
                // Kill the process group before draining the final bytes so
                // this informational probe never waits on inherited pipes.
                supervisor.kill_group();
                let Ok(status) = supervisor.reap() else {
                    return false;
                };
                // Output written just before exit is still in the pipe; read
                // it to EOF (bounded) before deciding, or the engine line can
                // be missed.
                let drain_deadline = Instant::now() + Duration::from_millis(100);
                let complete = loop {
                    match drain_registry_output(&mut stdout, &mut output) {
                        Ok(drain) if drain.truncated => break false,
                        Ok(drain) if drain.eof => break true,
                        Ok(_) if Instant::now() < drain_deadline => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Ok(_) | Err(_) => break false,
                    }
                };
                return complete && status.success() && registry_output_contains_engine(&output);
            }
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) | Err(_) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return false;
            }
        }
    }
}

fn registry_output_contains_engine(output: &[u8]) -> bool {
    output.len() <= MAX_IBUS_REGISTRY_OUTPUT_BYTES
        && String::from_utf8_lossy(output)
            .lines()
            .any(|line| line.trim() == ENGINE_NAME)
}

pub(crate) fn executable_in_path(name: &str) -> bool {
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

/// Component manifest names: the user installer writes `wayexpand.xml`, while
/// the deb, RPM and Arch packages install `wayexpand-ibus.xml`.
pub(crate) const COMPONENT_FILE_NAMES: [&str; 2] = ["wayexpand.xml", "wayexpand-ibus.xml"];

pub(super) fn component_file_present_in(directories: &[PathBuf]) -> bool {
    directories.iter().any(|directory| {
        COMPONENT_FILE_NAMES
            .iter()
            .any(|name| directory.join(name).is_file())
    })
}

/// Directories ibus-daemon loads component manifests from: every entry of
/// `IBUS_COMPONENT_PATH` when it is set (it replaces the default), otherwise
/// the distribution's data directory. IBus does not read
/// `$XDG_DATA_HOME/ibus/component`.
fn ibus_component_directories() -> Vec<PathBuf> {
    component_directories_for(env::var_os("IBUS_COMPONENT_PATH").as_deref())
}

pub(crate) fn component_directories_for(component_path: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    match component_path {
        Some(path) if !path.is_empty() => env::split_paths(path)
            .filter(|directory| directory.is_absolute())
            .collect(),
        _ => vec![
            PathBuf::from("/usr/share/ibus/component"),
            PathBuf::from("/usr/local/share/ibus/component"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    use std::{ffi::OsStr, path::PathBuf};

    use super::{
        component_directories_for, drain_registry_output, registry_command_contains_engine,
        registry_output_contains_engine, set_nonblocking, MAX_IBUS_REGISTRY_OUTPUT_BYTES,
    };

    #[test]
    fn registry_output_is_bounded_and_requires_the_engine_name() {
        assert!(registry_output_contains_engine(b"wayexpand\n"));
        assert!(!registry_output_contains_engine(b"other-engine\n"));
        // `--name-only` prints bare names; a longname or a lookalike engine
        // name must not count as registration.
        assert!(registry_output_contains_engine(b"xkb:us::eng\nwayexpand\n"));
        assert!(!registry_output_contains_engine(b"wayexpand-legacy\n"));
        assert!(!registry_output_contains_engine(
            b"  wayexpand - WayExpand\n"
        ));
        assert!(!registry_output_contains_engine(
            &[b'x'; MAX_IBUS_REGISTRY_OUTPUT_BYTES + 1]
        ));
    }

    #[test]
    fn component_directories_follow_ibus_daemon_search_rules() {
        let default = component_directories_for(None);
        assert!(default.contains(&PathBuf::from("/usr/share/ibus/component")));
        assert!(
            default
                .iter()
                .all(|directory| !directory.starts_with("/home")),
            "ibus-daemon never reads a per-user component directory: {default:?}"
        );
        assert_eq!(component_directories_for(Some(OsStr::new(""))), default);
        // IBUS_COMPONENT_PATH replaces the default list; relative entries
        // are ignored rather than resolved against our working directory.
        assert_eq!(
            component_directories_for(Some(OsStr::new("/opt/ibus:relative:/home/u/ibus"))),
            vec![PathBuf::from("/opt/ibus"), PathBuf::from("/home/u/ibus")]
        );
    }

    #[test]
    fn registry_drain_yields_for_continuous_output() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "yes wayexpand"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        set_nonblocking(&stdout).unwrap();
        let mut output = Vec::new();
        let started = Instant::now();
        let mut drain = super::RegistryDrain::default();
        while output.is_empty() && started.elapsed() < Duration::from_secs(1) {
            drain = drain_registry_output(&mut stdout, &mut output).unwrap();
            if output.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!output.is_empty());
        assert!(!drain.truncated || output.len() > MAX_IBUS_REGISTRY_OUTPUT_BYTES);
        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn registry_probe_kills_descendant_holding_stdout() {
        let started = Instant::now();
        assert!(registry_command_contains_engine(
            "/bin/sh",
            &["-c", "sleep 5 & printf 'wayexpand\\n'"],
            // Allow loaded CI runners time to schedule the parent shell while
            // still requiring the probe to finish well before the 5s child.
            Duration::from_secs(2),
        ));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "registry probe waited on an inherited stdout pipe: {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn registry_probe_bounds_continuous_output_and_time() {
        let started = Instant::now();
        assert!(!registry_command_contains_engine(
            "/bin/sh",
            &["-c", "while :; do printf 'wayexpand\\n'; done"],
            Duration::from_millis(500),
        ));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "continuous registry output exceeded its bounded probe: {:?}",
            started.elapsed()
        );
    }
}
