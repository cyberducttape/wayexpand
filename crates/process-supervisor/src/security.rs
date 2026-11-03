//! Read-only probes of restrictions effective in the calling process.

use serde::Serialize;
use std::{fs, os::unix::fs::MetadataExt, path::Path, sync::OnceLock};

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveSecuritySnapshot {
    pub pid: u32,
    pub namespaces: NamespaceSnapshot,
    pub no_new_privileges: Option<bool>,
    pub seccomp_mode: Option<u8>,
    pub seccomp_filters: Option<u32>,
    pub address_families: AddressFamilySnapshot,
    pub memory_write_execute_blocked: ProbeState,
    pub namespace_creation: ProbeState,
    pub personality_syscall: ProbeState,
    pub setuid_mode_bit: ProbeState,
    pub home_read_only: Option<bool>,
    pub system_read_only: Option<bool>,
    pub kernel_tunables_read_only: Option<bool>,
    pub proc_subset_pid: Option<bool>,
    pub proc_hidepid_invisible: Option<bool>,
    pub tmp_mount_id: Option<u64>,
    pub max_open_files: Option<(u64, u64)>,
    pub max_core_file_size: Option<(u64, u64)>,
    pub cgroup_memory_max_bytes: Option<u64>,
    pub cgroup_pids_max: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamespaceSnapshot {
    pub mount: Option<u64>,
    pub network: Option<u64>,
    pub user: Option<u64>,
    pub uts: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddressFamilySnapshot {
    pub unix: ProbeState,
    pub inet: ProbeState,
    pub inet6: ProbeState,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Allowed,
    Blocked,
    Unknown,
}

/// Probe the current process rather than inferring runtime protection from a
/// systemd unit file. The result is safe to publish in daemon and broker health
/// records; it contains no configuration or user data.
pub fn effective_security_snapshot() -> EffectiveSecuritySnapshot {
    static SNAPSHOT: OnceLock<EffectiveSecuritySnapshot> = OnceLock::new();
    SNAPSHOT.get_or_init(probe_effective_security).clone()
}

fn probe_effective_security() -> EffectiveSecuritySnapshot {
    let (no_new_privileges, seccomp_mode, seccomp_filters) = process_status();
    EffectiveSecuritySnapshot {
        pid: std::process::id(),
        namespaces: NamespaceSnapshot {
            mount: namespace_inode("mnt"),
            network: namespace_inode("net"),
            user: namespace_inode("user"),
            uts: namespace_inode("uts"),
        },
        no_new_privileges,
        seccomp_mode,
        seccomp_filters,
        address_families: AddressFamilySnapshot {
            unix: probe_socket(libc::AF_UNIX),
            inet: probe_socket(libc::AF_INET),
            inet6: probe_socket(libc::AF_INET6),
        },
        memory_write_execute_blocked: probe_write_execute_mapping(),
        namespace_creation: probe_user_namespace_creation(),
        personality_syscall: probe_personality_syscall(),
        setuid_mode_bit: probe_setuid_mode_bit(),
        home_read_only: std::env::var_os("HOME").and_then(|home| path_read_only(Path::new(&home))),
        system_read_only: path_read_only(Path::new("/usr")),
        kernel_tunables_read_only: path_read_only(Path::new("/proc/sys")),
        proc_subset_pid: proc_mount_option("subset=pid"),
        proc_hidepid_invisible: proc_mount_option("hidepid=2"),
        tmp_mount_id: mount_id_for(Path::new("/tmp")),
        max_open_files: process_limit("Max open files"),
        max_core_file_size: process_limit("Max core file size"),
        cgroup_memory_max_bytes: cgroup_limit("memory.max"),
        cgroup_pids_max: cgroup_limit("pids.max"),
    }
}

fn process_status() -> (Option<bool>, Option<u8>, Option<u32>) {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return (None, None, None);
    };
    let value = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
    };
    (
        value("NoNewPrivs:").and_then(|value| match value {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        }),
        value("Seccomp:").and_then(|value| value.parse().ok()),
        value("Seccomp_filters:").and_then(|value| value.parse().ok()),
    )
}

fn process_limit(label: &str) -> Option<(u64, u64)> {
    let limits = fs::read_to_string("/proc/self/limits").ok()?;
    let line = limits.lines().find(|line| line.starts_with(label))?;
    let mut values = line.strip_prefix(label)?.split_whitespace();
    Some((values.next()?.parse().ok()?, values.next()?.parse().ok()?))
}

fn cgroup_limit(name: &str) -> Option<u64> {
    let cgroup = fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let path = Path::new("/sys/fs/cgroup")
        .join(relative.trim_start_matches('/'))
        .join(name);
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn namespace_inode(name: &str) -> Option<u64> {
    fs::metadata(format!("/proc/self/ns/{name}"))
        .ok()
        .map(|metadata| metadata.ino())
}

fn probe_socket(domain: libc::c_int) -> ProbeState {
    // SAFETY: socket creates a descriptor or returns -1; the successful
    // descriptor is immediately closed and no connection is made.
    let descriptor = unsafe { libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if descriptor < 0 {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EAFNOSUPPORT)
            | Some(libc::EPROTONOSUPPORT)
            | Some(libc::EPERM)
            | Some(libc::EACCES) => ProbeState::Blocked,
            _ => ProbeState::Unknown,
        }
    } else {
        // SAFETY: descriptor was returned by socket above.
        unsafe { libc::close(descriptor) };
        ProbeState::Allowed
    }
}

fn probe_write_execute_mapping() -> ProbeState {
    // SAFETY: this requests one anonymous page and does not access the mapping.
    // A successful mapping is immediately removed.
    let mapping = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapping == libc::MAP_FAILED {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) | Some(libc::EACCES) => ProbeState::Blocked,
            _ => ProbeState::Unknown,
        }
    } else {
        // SAFETY: mapping is a successful one-page mmap result.
        unsafe { libc::munmap(mapping, 4096) };
        ProbeState::Allowed
    }
}

fn probe_personality_syscall() -> ProbeState {
    // SAFETY: personality(u32::MAX) queries current state without changing it.
    let result = unsafe { libc::personality(0xffff_ffff_u32 as libc::c_ulong) };
    if result >= 0 {
        ProbeState::Allowed
    } else if matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM) | Some(libc::EACCES)
    ) {
        ProbeState::Blocked
    } else {
        ProbeState::Unknown
    }
}

fn probe_user_namespace_creation() -> ProbeState {
    let mut pipe = [0; 2];
    // SAFETY: pipe2 initializes the two-element descriptor array.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return ProbeState::Unknown;
    }
    // SAFETY: fork returns in parent and child. The child below uses only
    // async-signal-safe syscalls and immediately exits.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: close the read end, perform the one namespace probe, report
        // one byte, then exit without running destructors or touching locks.
        unsafe {
            libc::close(pipe[0]);
            let result = libc::unshare(libc::CLONE_NEWUSER);
            let outcome = if result == 0 {
                1u8
            } else {
                let errno = *libc::__errno_location();
                if errno == libc::EPERM
                    || errno == libc::EACCES
                    || errno == libc::EINVAL
                    || errno == libc::ENOSYS
                {
                    0
                } else {
                    2
                }
            };
            libc::write(pipe[1], (&outcome as *const u8).cast(), 1);
            libc::_exit(0);
        }
    }
    // SAFETY: parent closes the write end and reads one byte from the pipe.
    unsafe { libc::close(pipe[1]) };
    if pid < 0 {
        // SAFETY: pipe[0] remains owned by this function.
        unsafe { libc::close(pipe[0]) };
        return ProbeState::Unknown;
    }
    let mut outcome = 2u8;
    // SAFETY: outcome points to one writable byte and pipe[0] is a valid fd.
    let bytes = unsafe { libc::read(pipe[0], (&mut outcome as *mut u8).cast(), 1) };
    // SAFETY: pipe[0] is owned by this function.
    unsafe { libc::close(pipe[0]) };
    // SAFETY: pid is the child created above; waitpid reaps it.
    let mut status = 0;
    let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
    if bytes != 1 || waited != pid || !libc::WIFEXITED(status) {
        return ProbeState::Unknown;
    }
    match outcome {
        0 => ProbeState::Blocked,
        1 => ProbeState::Allowed,
        _ => ProbeState::Unknown,
    }
}

#[cfg(target_os = "linux")]
fn probe_setuid_mode_bit() -> ProbeState {
    let name = c"wayexpand-security-probe";
    // SAFETY: creates an anonymous memory-backed file, which is closed below.
    let descriptor = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if descriptor < 0 {
        return ProbeState::Unknown;
    }
    // SAFETY: descriptor is the open memfd created above; the ephemeral inode
    // is not executable and is removed when its final descriptor is closed.
    let result = unsafe { libc::fchmod(descriptor, 0o4755) };
    let error = (result < 0).then(std::io::Error::last_os_error);
    // SAFETY: descriptor is still owned here.
    unsafe { libc::close(descriptor) };
    match (result, error.and_then(|error| error.raw_os_error())) {
        (0, _) => ProbeState::Allowed,
        (_, Some(libc::EPERM)) | (_, Some(libc::EACCES)) => ProbeState::Blocked,
        _ => ProbeState::Unknown,
    }
}

#[cfg(not(target_os = "linux"))]
fn probe_setuid_mode_bit() -> ProbeState {
    ProbeState::Unknown
}

fn path_read_only(path: &Path) -> Option<bool> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs initializes the supplied struct for a NUL-terminated path.
    let mut stats = unsafe { std::mem::zeroed::<libc::statvfs>() };
    // SAFETY: path and stats are valid for this call.
    (unsafe { libc::statvfs(path.as_ptr(), &mut stats) } == 0)
        .then_some(stats.f_flag & libc::ST_RDONLY != 0)
}

fn proc_mount_option(option: &str) -> Option<bool> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let proc_mount = mountinfo.lines().find(|line| {
        let mut fields = line.split_whitespace();
        fields.nth(4) == Some("/proc")
    })?;
    Some(proc_mount.split_whitespace().any(|field| field == option))
}

fn mount_id_for(path: &Path) -> Option<u64> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let target = path.to_str()?;
    mountinfo.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let id = fields.next()?.parse().ok()?;
        fields.next()?;
        fields.next()?;
        fields.next()?;
        let mountpoint = fields.next()?;
        (mountpoint == target).then_some(id)
    })
}

#[cfg(test)]
mod tests {
    use super::effective_security_snapshot;

    #[test]
    fn effective_snapshot_reports_live_kernel_state_without_user_data() {
        let snapshot = effective_security_snapshot();
        assert_eq!(snapshot.pid, std::process::id());
        assert!(snapshot.namespaces.mount.is_some());
        assert!(snapshot.seccomp_mode.is_some());
        assert!(snapshot.no_new_privileges.is_some());
        assert!(snapshot.tmp_mount_id.is_some());
        assert!(snapshot.max_open_files.is_some());
    }
}
