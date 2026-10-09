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
    let Some(mut stdout) = child.stdout.take() else {
        return false;
    };
    if set_nonblocking(&stdout).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut output = Vec::new();
    loop {
        let drain = match drain_registry_output(&mut stdout, &mut output) {
            Ok(drain) => drain,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        };
        if drain.truncated {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
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
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn registry_output_contains_engine(output: &[u8]) -> bool {
    output.len() <= MAX_IBUS_REGISTRY_OUTPUT_BYTES
        && String::from_utf8_lossy(output)
            .lines()
            .any(|line| line.contains("wayexpand"))
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

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    use super::{
        drain_registry_output, registry_output_contains_engine, set_nonblocking,
        MAX_IBUS_REGISTRY_OUTPUT_BYTES,
    };

    #[test]
    fn registry_output_is_bounded_and_requires_the_engine_name() {
        assert!(registry_output_contains_engine(b"wayexpand\n"));
        assert!(!registry_output_contains_engine(b"other-engine\n"));
        assert!(!registry_output_contains_engine(
            &[b'x'; MAX_IBUS_REGISTRY_OUTPUT_BYTES + 1]
        ));
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
}
