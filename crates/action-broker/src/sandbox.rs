//! Per-action Landlock filesystem confinement.
//!
//! Rulesets are prepared in the broker before spawn. The child pre-exec hook
//! performs only `prctl` and `landlock_restrict_self`, then execs the pinned
//! program. A requested sandbox is never silently skipped.

#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, OwnedFd};

#[cfg(target_os = "linux")]
pub(crate) fn prepare(
    sandbox: &crate::config::ActionSandbox,
    program: &str,
    cwd: Option<&str>,
) -> Result<OwnedFd, String> {
    use landlock::{Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, ABI};
    use std::path::Path;

    let version = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            1u32, // LANDLOCK_CREATE_RULESET_VERSION
        )
    };
    if version < 1 {
        return Err(format!(
            "Landlock is unavailable: {}",
            std::io::Error::last_os_error()
        ));
    }
    let abi = match version.min(9) {
        1 => ABI::V1,
        2 => ABI::V2,
        3 => ABI::V3,
        4 => ABI::V4,
        5 => ABI::V5,
        6 => ABI::V6,
        7 => ABI::V7,
        8 => ABI::V8,
        _ => ABI::V9,
    };
    let all_rights = AccessFs::from_all(abi);
    let read_rights = AccessFs::from_read(abi);
    let write_rights = AccessFs::from_write(abi) & !AccessFs::IoctlDev & !AccessFs::ResolveUnix;

    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(all_rights)
        .map_err(|error| format!("could not configure Landlock filesystem rules: {error}"))?;
    let mut created = ruleset
        .create()
        .map_err(|error| format!("could not create Landlock ruleset: {error}"))?;

    // Read-only runtime paths let dynamically linked programs and interpreters
    // start. More specific application data must be explicitly granted below.
    for path in [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/proc",
        "/etc/ld.so.cache",
    ] {
        if Path::new(path).exists() {
            created = add_path_rule(created, path, read_rights, abi, "runtime")?;
        }
    }
    for path in sandbox
        .read_paths
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(program))
        .chain(cwd)
    {
        created = add_path_rule(created, path, read_rights, abi, "read")?;
    }
    for path in &sandbox.write_paths {
        created = add_path_rule(created, path, write_rights, abi, "write")?;
    }

    Option::<OwnedFd>::from(created)
        .ok_or_else(|| "Landlock ruleset was not created by the running kernel".to_string())
}

#[cfg(target_os = "linux")]
fn add_path_rule(
    ruleset: landlock::RulesetCreated,
    path: &str,
    access: landlock::BitFlags<landlock::AccessFs>,
    abi: landlock::ABI,
    label: &str,
) -> Result<landlock::RulesetCreated, String> {
    use landlock::{AccessFs, PathBeneath, PathFd, RulesetCreatedAttr};
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("could not inspect {label} path '{path}': {error}"))?;
    let access = if metadata.is_dir() {
        access
    } else {
        access & AccessFs::from_file(abi)
    };
    ruleset
        .add_rule(PathBeneath::new(
            PathFd::new(path).map_err(|error| error.to_string())?,
            access,
        ))
        .map_err(|error| format!("could not allow {label} path '{path}': {error}"))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn prepare(
    _sandbox: &crate::config::ActionSandbox,
    _program: &str,
    _cwd: Option<&str>,
) -> Result<(), String> {
    Err("per-action filesystem sandboxing requires Linux Landlock".to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn configure_command(command: &mut std::process::Command, ruleset: &OwnedFd) {
    use std::os::unix::process::CommandExt;

    let fd = ruleset.as_raw_fd();
    // SAFETY: the hook performs only async-signal-safe syscalls after fork.
    // The ruleset descriptor is CLOEXEC: it is available to restrict_self,
    // then closes automatically when the target program is executed.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn configure_command(_command: &mut std::process::Command, _ruleset: &()) {}
