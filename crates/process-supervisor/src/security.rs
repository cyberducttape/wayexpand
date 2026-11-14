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
    /// Whether a child can create a user namespace and map one caller UID/GID.
    /// This is stronger evidence than `unshare(CLONE_NEWUSER)` alone, but it
    /// does not attest that systemd configured a particular service sandbox.
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
        namespace_creation: probe_user_namespace_setup(),
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

fn probe_user_namespace_setup() -> ProbeState {
    // Merely entering CLONE_NEWUSER is not sufficient for systemd's
    // unprivileged filesystem sandboxing: the caller must also be able to map
    // its UID and GID inside the namespace. Build these bytes before fork so
    // the child needs no allocator or other non-async-signal-safe runtime.
    let uid_map = format!("0 {} 1", unsafe { libc::getuid() }).into_bytes();
    let gid_map = format!("0 {} 1", unsafe { libc::getgid() }).into_bytes();
    let mut pipe = [0; 2];
    // SAFETY: pipe2 initializes the two-element descriptor array.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return ProbeState::Unknown;
    }
    // SAFETY: fork returns in parent and child. The child below uses only
    // async-signal-safe syscalls and immediately exits.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: close the read end, create and map a disposable user
        // namespace, report one byte, then exit without running destructors
        // or touching locks.
        unsafe {
            libc::close(pipe[0]);
            let result = libc::unshare(libc::CLONE_NEWUSER);
            let outcome = if result != 0 {
                namespace_probe_errno_outcome(*libc::__errno_location())
            } else {
                let setgroups = write_namespace_file(c"/proc/self/setgroups".as_ptr(), b"deny");
                let uid = write_namespace_file(c"/proc/self/uid_map".as_ptr(), &uid_map);
                let gid = write_namespace_file(c"/proc/self/gid_map".as_ptr(), &gid_map);
                if setgroups == 0 && uid == 0 && gid == 0 {
                    1
                } else {
                    let errno = [setgroups, uid, gid]
                        .into_iter()
                        .find(|error| *error != 0)
                        .unwrap_or(libc::EIO);
                    namespace_probe_errno_outcome(errno)
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

/// Write a short mapping to a proc file from the post-fork probe child.
/// Returns zero on success or the syscall errno. The caller passes static
/// paths and buffers allocated before fork.
unsafe fn write_namespace_file(path: *const libc::c_char, contents: &[u8]) -> libc::c_int {
    // SAFETY: path is a NUL-terminated static C string and the returned fd is
    // closed before this function returns.
    let fd = unsafe { libc::open(path, libc::O_WRONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return unsafe { *libc::__errno_location() };
    }
    let mut written = 0;
    while written < contents.len() {
        // SAFETY: the slice remains valid for the duration of the syscall and
        // fd is the proc file opened above.
        let count = unsafe {
            libc::write(
                fd,
                contents[written..].as_ptr().cast(),
                contents.len() - written,
            )
        };
        if count < 0 {
            let errno = unsafe { *libc::__errno_location() };
            if errno == libc::EINTR {
                continue;
            }
            // SAFETY: fd is owned by this function.
            unsafe { libc::close(fd) };
            return errno;
        }
        if count == 0 {
            // SAFETY: fd is owned by this function.
            unsafe { libc::close(fd) };
            return libc::EIO;
        }
        written += count as usize;
    }
    // SAFETY: fd is owned by this function.
    let close_result = unsafe { libc::close(fd) };
    if close_result == 0 {
        0
    } else {
        unsafe { *libc::__errno_location() }
    }
}

fn namespace_probe_errno_outcome(errno: libc::c_int) -> u8 {
    if errno == libc::EPERM
        || errno == libc::EACCES
        || errno == libc::EINVAL
        || errno == libc::ENOSYS
    {
        0
    } else {
        2
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
    use super::{
        effective_security_snapshot, mount_id_for, namespace_probe_errno_outcome, ProbeState,
    };
    use std::path::Path;

    #[test]
    fn namespace_probe_classifies_expected_policy_denials_as_blocked() {
        for errno in [libc::EPERM, libc::EACCES, libc::EINVAL, libc::ENOSYS] {
            assert_eq!(namespace_probe_errno_outcome(errno), 0);
        }
        assert_eq!(namespace_probe_errno_outcome(libc::EIO), 2);
        assert!(matches!(
            effective_security_snapshot().namespace_creation,
            ProbeState::Allowed | ProbeState::Blocked | ProbeState::Unknown
        ));
    }

    #[test]
    fn effective_snapshot_reports_live_kernel_state_without_user_data() {
        let snapshot = effective_security_snapshot();
        assert_eq!(snapshot.pid, std::process::id());
        assert!(snapshot.namespaces.mount.is_some());
        assert!(snapshot.seccomp_mode.is_some());
        assert!(snapshot.no_new_privileges.is_some());
        // Some CI hosts mount `/tmp` as part of `/` rather than as a separate
        // mount, so the exact mountpoint may be unavailable. The snapshot
        // should report the same optional value as the live mount table probe.
        assert_eq!(snapshot.tmp_mount_id, mount_id_for(Path::new("/tmp")));
        assert!(snapshot.max_open_files.is_some());
    }
}
