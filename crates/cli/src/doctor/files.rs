//! Configuration file, parent directory, and control-socket ownership and permission checks.

use crate::*;

pub(crate) fn existing_control_socket_is_healthy(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    control_socket_parent_is_healthy(path)
        && metadata.file_type().is_socket()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.mode() & 0o077 == 0
}

pub(crate) fn control_socket_parent_is_healthy(path: &Path) -> bool {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return false;
    };
    let Ok(resolved_parent) = fs::canonicalize(parent) else {
        return false;
    };
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = resolved_parent.as_path();
    let mut immediate = true;
    loop {
        let Ok(metadata) = fs::metadata(current) else {
            return false;
        };
        if !metadata.is_dir() {
            return false;
        }
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            return false;
        }
        let mode = metadata.mode() & 0o7777;
        let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
        if mode & 0o022 != 0 && (!root_sticky || immediate) {
            return false;
        }
        if current == Path::new("/") {
            return true;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
        immediate = false;
    }
}

pub(crate) fn print_config_diagnostics(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => {
            let mode = metadata.mode() & 0o777;
            println!(
                "Config: {} (mode {:04o}, uid {})",
                path.display(),
                mode,
                metadata.uid()
            );
            if !metadata.file_type().is_file() {
                println!("Config warning: path is not a regular file");
            }
            let current_uid = rustix::process::geteuid().as_raw();
            if metadata.uid() != current_uid && metadata.uid() != 0 {
                println!("Config warning: file is not owned by the current user or root");
            }
            if mode & 0o022 != 0 {
                println!("Config warning: file is writable by group or other users");
            }
            print_config_parent_diagnostics(path);
            match Config::load(path) {
                Ok(_) => {
                    println!("Config validation: OK");
                    true
                }
                Err(error) => {
                    println!("Config validation: FAILED ({})", error.safe_summary());
                    false
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Config: {} (not found)", path.display());
            false
        }
        Err(error) => {
            println!("Config: {} (unreadable: {error})", path.display());
            false
        }
    }
}

pub(crate) fn print_config_parent_diagnostics(path: &Path) {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut current = resolved
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    loop {
        let Ok(metadata) = fs::metadata(current) else {
            return;
        };
        if !metadata.is_dir() {
            println!("Config warning: parent is not a directory");
            return;
        }
        let current_uid = rustix::process::geteuid().as_raw();
        let mode = metadata.mode() & 0o7777;
        // Sticky protection limits unlink/rename rights but does not make an
        // untrusted directory a valid configuration ancestor. Keep doctor in
        // lockstep with Config::load's security policy.
        if metadata.uid() != current_uid && metadata.uid() != 0 {
            println!(
                "Config warning: parent {} is owned by untrusted uid {}",
                current.display(),
                metadata.uid()
            );
        }
        if mode & 0o022 != 0 && mode & 0o1000 == 0 {
            println!(
                "Config warning: parent {} is writable by group or other users without sticky protection (mode {:04o})",
                current.display(),
                mode & 0o7777
            );
            println!("Config fix: chmod go-w {}", current.display());
        }
        if current == Path::new("/") {
            return;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
}

pub(crate) fn print_control_socket_diagnostics() -> bool {
    let Some(path) = std::env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
    else {
        println!("Control socket: disabled (XDG_RUNTIME_DIR unavailable)");
        return true;
    };

    let mut valid = true;
    println!("Control socket: {}", path.display());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let resolved_parent = match fs::canonicalize(parent) {
        Ok(resolved) => Some(resolved),
        Err(error) => {
            println!("Control socket warning: parent is unavailable ({error})");
            valid = false;
            None
        }
    };
    if let Some(resolved_parent) = resolved_parent.as_deref() {
        let current_uid = rustix::process::geteuid().as_raw();
        let mut current = resolved_parent;
        let mut immediate = true;
        loop {
            match fs::metadata(current) {
                Ok(metadata) if metadata.is_dir() => {
                    if metadata.uid() != current_uid && metadata.uid() != 0 {
                        println!(
                            "Control socket warning: {} is not owned by the current user or root",
                            if immediate { "parent" } else { "an ancestor" }
                        );
                        valid = false;
                    }
                    let mode = metadata.mode() & 0o7777;
                    let root_sticky = metadata.uid() == 0 && mode & 0o1000 != 0;
                    if mode & 0o022 != 0 && (!root_sticky || immediate) {
                        println!(
                            "Control socket warning: {} is writable by group or other users",
                            if immediate { "parent" } else { "an ancestor" }
                        );
                        valid = false;
                    }
                }
                Ok(_) => {
                    println!("Control socket warning: parent is not a directory");
                    valid = false;
                    break;
                }
                Err(error) => {
                    println!("Control socket warning: parent is unavailable ({error})");
                    valid = false;
                    break;
                }
            }
            if current == Path::new("/") {
                break;
            }
            current = current.parent().unwrap_or_else(|| Path::new("/"));
            immediate = false;
        }
    }
    let existing_path = resolved_parent
        .as_deref()
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.clone());

    match fs::symlink_metadata(&existing_path) {
        Ok(metadata) => {
            let mode = metadata.mode() & 0o777;
            println!(
                "Control socket existing path: mode {:04o}, uid {}, socket={}",
                mode,
                metadata.uid(),
                metadata.file_type().is_socket()
            );
            if !metadata.file_type().is_socket() {
                println!("Control socket warning: existing path is not a socket");
                valid = false;
            }
            if metadata.uid() != rustix::process::geteuid().as_raw() {
                println!("Control socket warning: existing path is not owned by the current user");
                valid = false;
            }
            if mode & 0o077 != 0 {
                println!("Control socket warning: existing socket is more permissive than 0600");
                valid = false;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Control socket existing path: not present (will be created)");
            valid = false;
        }
        Err(error) => {
            println!("Control socket existing path: unreadable ({error})");
            valid = false;
        }
    }
    valid
}
