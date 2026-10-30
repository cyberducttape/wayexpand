//! Per-action Landlock filesystem and optional seccomp network confinement.
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
pub(crate) fn configure_command_with_network(
    command: &mut std::process::Command,
    ruleset: &OwnedFd,
    deny_network: bool,
) -> Result<(), String> {
    let fd = ruleset.as_raw_fd();
    let network_filter = deny_network.then(build_network_filter).transpose()?;
    configure_command_with_network_filter(command, fd, network_filter);
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_command_with_network_filter(
    command: &mut std::process::Command,
    fd: std::os::fd::RawFd,
    network_filter: Option<Vec<libc::sock_filter>>,
) {
    use std::os::unix::process::CommandExt;

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
            if let Some(filter) = &network_filter {
                let program = libc::sock_fprog {
                    len: filter.len() as u16,
                    filter: filter.as_ptr() as *mut libc::sock_filter,
                };
                const SECCOMP_SET_MODE_FILTER: libc::c_uint = 1;
                if libc::syscall(
                    libc::SYS_seccomp,
                    SECCOMP_SET_MODE_FILTER,
                    0u32,
                    &program as *const libc::sock_fprog,
                ) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

#[cfg(target_os = "linux")]
fn build_network_filter() -> Result<Vec<libc::sock_filter>, String> {
    use libc::{sock_filter, BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W};

    // Reject ABI switching before checking syscall numbers. This avoids a
    // 32-bit compatibility syscall or x32 syscall bypassing native numbers.
    let audit_arch = match std::env::consts::ARCH {
        "x86_64" => 0xc000_003e,
        "aarch64" => 0xc000_00b7,
        "riscv64" => 0xc000_00f3,
        architecture => {
            return Err(format!(
                "per-action network filtering is unsupported on {architecture}"
            ));
        }
    };
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_DATA_NR_OFFSET: u32 = 0;
    const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
    const SECCOMP_DATA_ARG0_OFFSET: u32 = 16;
    const AF_UNIX_VALUE: u32 = 1;
    const EPERM_VALUE: u32 = libc::EPERM as u32;

    let statement = |code, k| sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    };
    let jump = |k, jt, jf| sock_filter {
        code: (BPF_JMP | BPF_JEQ | BPF_K) as u16,
        jt,
        jf,
        k,
    };
    let deny = || statement(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM_VALUE);
    let mut filter = vec![
        statement(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_ARCH_OFFSET),
        jump(audit_arch, 1, 0),
        statement(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR_OFFSET),
    ];

    // On x86_64, reject x32's syscall-number bit even though it shares the
    // AUDIT_ARCH_X86_64 value. Other supported architectures have no x32 ABI.
    if std::env::consts::ARCH == "x86_64" {
        use libc::BPF_JSET;
        filter.push(sock_filter {
            code: (BPF_JMP | BPF_JSET | BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: 0x4000_0000,
        });
        filter.push(statement(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));
        filter.push(statement(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR_OFFSET));
    }

    // AF_UNIX remains available for local IPC. All other socket families,
    // including IPv4/IPv6, packet, and netlink, fail with EPERM. The child is
    // spawned with only stdio pipes/null and the pinned executable descriptor,
    // so it receives no pre-opened network socket.
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        filter.push(jump(syscall as u32, 0, 4));
        filter.push(statement(
            BPF_LD | BPF_W | BPF_ABS,
            SECCOMP_DATA_ARG0_OFFSET,
        ));
        filter.push(jump(AF_UNIX_VALUE, 1, 0));
        filter.push(deny());
        filter.push(statement(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR_OFFSET));
    }

    // io_uring can create sockets through IORING_OP_SOCKET without calling
    // socket(2), so deny creation of an io_uring instance in this profile.
    filter.push(jump(libc::SYS_io_uring_setup as u32, 0, 1));
    filter.push(deny());
    filter.push(statement(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    if filter.len() > u16::MAX as usize {
        return Err("per-action network filter exceeds the kernel instruction limit".into());
    }
    Ok(filter)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn filter_has_a_native_architecture_guard() {
        let filter = build_network_filter().unwrap();
        assert!(filter.len() >= 7);
        // First instruction loads seccomp_data.arch; the next comparison
        // branches to a kill action for an ABI mismatch.
        assert_eq!(filter[0].k, 4);
        assert_eq!(filter[1].jt, 1);
        assert_eq!(filter[2].k, 0x8000_0000);
    }

    #[test]
    #[ignore = "invoked by ActionExecutor with a per-action sandbox"]
    fn action_executor_network_probe_child() {
        for family in [
            libc::AF_INET,
            libc::AF_INET6,
            libc::AF_PACKET,
            libc::AF_NETLINK,
        ] {
            let descriptor =
                unsafe { libc::socket(family, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
            assert_eq!(descriptor, -1, "network socket family {family} was allowed");
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            );
        }
        let unix_socket =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(
            unix_socket >= 0,
            "local Unix sockets should remain available"
        );
        unsafe { libc::close(unix_socket) };
        let io_uring = unsafe {
            libc::syscall(
                libc::SYS_io_uring_setup,
                1u32,
                std::ptr::null_mut::<libc::c_void>(),
            )
        };
        assert_eq!(io_uring, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn configure_command_with_network(
    _command: &mut std::process::Command,
    _ruleset: &(),
    _deny_network: bool,
) -> Result<(), String> {
    Err("per-action filesystem sandboxing requires Linux Landlock".into())
}
