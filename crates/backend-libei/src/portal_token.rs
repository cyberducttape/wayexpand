use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(crate) const PORTAL_TOKEN_FILENAME: &str = "libei-portal-token";
pub(crate) const MAX_PORTAL_TOKEN_BYTES: usize = 4096;
const PORTAL_TOKEN_TEMP_ATTEMPTS: usize = 16;

/// Get the path where portal session tokens are stored.
/// Returns None if XDG_CONFIG_HOME is not set and home directory cannot be determined.
pub fn portal_token_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WAYEXPAND_PORTAL_TOKEN_PATH") {
        let path = PathBuf::from(path);
        return path.is_absolute().then_some(path);
    }
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let config_home = PathBuf::from(config_home);
        if config_home.is_absolute() {
            let mut path = config_home;
            path.push("wayexpand");
            path.push(PORTAL_TOKEN_FILENAME);
            return Some(path);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".config/wayexpand");
        path.push(PORTAL_TOKEN_FILENAME);
        return Some(path);
    }
    None
}

pub fn reset_portal_token() -> std::io::Result<bool> {
    let path = match secure_portal_token_path(false) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    reset_portal_token_at(&path)
}

pub(crate) fn reset_portal_token_at(path: &Path) -> std::io::Result<bool> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_token_metadata(path, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_portal_token_if_enabled(
    persist: bool,
    explicit_path: Option<&Path>,
) -> std::io::Result<Option<String>> {
    if !persist {
        return Ok(None);
    }
    let path = match explicit_path {
        Some(path) => secure_portal_token_path_for(path, false)?,
        None => secure_portal_token_path(false)?,
    };
    read_portal_token_at(&path)
}

pub(crate) fn read_portal_token_at(path: &Path) -> std::io::Result<Option<String>> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    let mut file = OpenOptions::new();
    file.read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = match file.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    validate_token_metadata(path, &metadata)?;
    if metadata.len() > MAX_PORTAL_TOKEN_BYTES as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is too large",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_PORTAL_TOKEN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PORTAL_TOKEN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is too large",
        ));
    }
    let token = String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is not valid UTF-8",
        )
    })?;
    if token.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "portal restoration token is empty",
        ));
    }
    Ok(Some(token))
}

pub(crate) fn store_portal_token_if_enabled(
    persist: bool,
    explicit_path: Option<&Path>,
    token: &str,
) -> std::io::Result<()> {
    if !persist {
        return Ok(());
    }
    if let Some(path) = explicit_path {
        let path = secure_portal_token_path_for(path, true)?;
        return store_portal_token_at(&path, token);
    }
    store_portal_token(token)
}

fn store_portal_token(token: &str) -> std::io::Result<()> {
    if token.is_empty() || token.len() > MAX_PORTAL_TOKEN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal restoration token has an invalid size",
        ));
    }
    if token.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal restoration token contains NUL",
        ));
    }

    let path = secure_portal_token_path(true)?;
    store_portal_token_at(&path, token)
}

pub(crate) fn store_portal_token_at(path: &Path, token: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    validate_token_parent_chain(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_token_metadata(path, &metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut temporary = None;
    let mut file = None;
    for attempt in 0..PORTAL_TOKEN_TEMP_ATTEMPTS {
        let candidate = parent.join(format!(
            ".{PORTAL_TOKEN_FILENAME}.tmp.{}.{}",
            std::process::id(),
            attempt
        ));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        match options.open(&candidate) {
            Ok(created) => {
                temporary = Some(candidate);
                file = Some(created);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let temporary = temporary.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique portal token temporary file",
        )
    })?;
    let result = (|| -> std::io::Result<()> {
        let mut file = file.ok_or_else(|| {
            std::io::Error::other("portal token temporary file was not opened")
        })?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn secure_portal_token_path(create_parent: bool) -> std::io::Result<PathBuf> {
    let raw_path = portal_token_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot determine config directory for portal token",
        )
    })?;
    secure_portal_token_path_for(&raw_path, create_parent)
}

fn secure_portal_token_path_for(raw_path: &Path, create_parent: bool) -> std::io::Result<PathBuf> {
    let raw_parent = raw_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no parent directory",
        )
    })?;
    let parent = trusted_token_parent(raw_parent, create_parent)?;
    let filename = raw_path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "portal token path has no file name",
        )
    })?;
    Ok(parent.join(filename))
}

fn trusted_token_parent(path: &Path, create: bool) -> std::io::Result<PathBuf> {
    if create {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "portal token parent is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(path)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
            Err(error) => return Err(error),
        }
    }
    let resolved = fs::canonicalize(path)?;
    validate_token_parent_chain(&resolved)?;
    Ok(resolved)
}

pub(crate) fn validate_token_parent_chain(path: &Path) -> std::io::Result<()> {
    let current_uid = rustix::process::geteuid().as_raw();
    let mut current = path;
    loop {
        let metadata = fs::symlink_metadata(current)?;
        if !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "portal token parent is not a directory",
            ));
        }
        let is_system_dir = current == Path::new("/")
            || current == Path::new("/home")
            || current == Path::new("/run");
        if !is_system_dir && metadata.uid() != current_uid && metadata.uid() != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "portal token parent has an untrusted owner",
            ));
        }
        let mode = metadata.permissions().mode() & 0o7777;
        if !token_parent_mode_is_secure(mode) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "portal token parent is writable by group or other users",
            ));
        }
        if current == Path::new("/") {
            break;
        }
        current = current.parent().unwrap_or_else(|| Path::new("/"));
    }
    Ok(())
}

pub(crate) fn token_parent_mode_is_secure(mode: u32) -> bool {
    mode & 0o022 == 0 || mode & 0o1000 != 0
}

fn validate_token_metadata(path: &Path, metadata: &fs::Metadata) -> std::io::Result<()> {
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("portal token is not a regular file: {}", path.display()),
        ));
    }
    let current_uid = rustix::process::geteuid().as_raw();
    if metadata.uid() != current_uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "portal token has an untrusted owner",
        ));
    }
    if metadata.permissions().mode() & 0o7777 != 0o600 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "portal token permissions must be 0600",
        ));
    }
    Ok(())
}
