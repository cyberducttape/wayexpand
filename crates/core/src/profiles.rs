//! Exact focused-application to local profile assignments.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

const MAX_ASSIGNMENT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileAssignments {
    /// Lowercase exact desktop application IDs, never globs or titles.
    #[serde(default)]
    pub applications: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ProfileAssignmentsError {
    #[error("profile assignment file must be a private user-owned regular file")]
    InsecureFile,
    #[error("profile assignment directory must be a private user-owned real directory")]
    InsecureDirectory,
    #[error("profile assignment file exceeds {MAX_ASSIGNMENT_BYTES} bytes")]
    TooLarge,
    #[error("profile assignments are invalid: {0}")]
    Invalid(String),
    #[error("could not read profile assignments: {0}")]
    Read(#[source] std::io::Error),
    #[error("could not write profile assignments: {0}")]
    Write(#[source] std::io::Error),
}

impl ProfileAssignments {
    /// Load a bounded, private mapping. A missing mapping is an empty mapping;
    /// malformed or insecure state is an error so the daemon cannot guess.
    pub fn load(path: &Path) -> Result<Self, ProfileAssignmentsError> {
        let Some(parent) = path.parent() else {
            return Err(ProfileAssignmentsError::InsecureDirectory);
        };
        if matches!(fs::symlink_metadata(parent), Err(ref error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(Self::default());
        }
        validate_profile_directory(parent)?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(ProfileAssignmentsError::Read(error)),
        };
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o777 != 0o600
            || metadata.len() > MAX_ASSIGNMENT_BYTES
        {
            return Err(if metadata.len() > MAX_ASSIGNMENT_BYTES {
                ProfileAssignmentsError::TooLarge
            } else {
                ProfileAssignmentsError::InsecureFile
            });
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(ProfileAssignmentsError::Read)?;
        let opened = file.metadata().map_err(ProfileAssignmentsError::Read)?;
        if !opened.file_type().is_file()
            || opened.uid() != rustix::process::geteuid().as_raw()
            || opened.mode() & 0o777 != 0o600
            || opened.len() > MAX_ASSIGNMENT_BYTES
        {
            return Err(ProfileAssignmentsError::InsecureFile);
        }
        let mut source = String::new();
        file.take(MAX_ASSIGNMENT_BYTES + 1)
            .read_to_string(&mut source)
            .map_err(ProfileAssignmentsError::Read)?;
        if source.len() as u64 > MAX_ASSIGNMENT_BYTES {
            return Err(ProfileAssignmentsError::TooLarge);
        }
        let mut assignments: Self = toml::from_str(&source)
            .map_err(|error| ProfileAssignmentsError::Invalid(error.to_string()))?;
        assignments.validate_and_normalize()?;
        Ok(assignments)
    }

    pub fn validate_and_normalize(&mut self) -> Result<(), ProfileAssignmentsError> {
        let mut normalized = BTreeMap::new();
        for (app_id, profile) in std::mem::take(&mut self.applications) {
            let app_id = Self::normalize_application_id(&app_id)?;
            validate_profile_name(&profile)?;
            if normalized.insert(app_id.clone(), profile).is_some() {
                return Err(ProfileAssignmentsError::Invalid(format!(
                    "application ID {app_id} is assigned more than once"
                )));
            }
        }
        self.applications = normalized;
        Ok(())
    }

    pub fn normalize_application_id(app_id: &str) -> Result<String, ProfileAssignmentsError> {
        let normalized = app_id.to_ascii_lowercase();
        if normalized.is_empty()
            || normalized.len() > 255
            || !normalized.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(ProfileAssignmentsError::Invalid(
                "application IDs must be exact desktop IDs containing only letters, digits, dot, underscore, or hyphen".into(),
            ));
        }
        Ok(normalized)
    }

    pub fn profile_for_app(&self, app_id: Option<&str>) -> Option<&str> {
        let app_id = app_id?.to_ascii_lowercase();
        self.applications.get(&app_id).map(String::as_str)
    }

    /// Write through a private temporary file, then atomically replace the
    /// mapping so the daemon sees either the old or complete new table.
    pub fn save(&self, path: &Path) -> Result<(), ProfileAssignmentsError> {
        let mut validated = self.clone();
        validated.validate_and_normalize()?;
        let text = toml::to_string_pretty(&validated)
            .map_err(|error| ProfileAssignmentsError::Invalid(error.to_string()))?;
        if text.len() as u64 > MAX_ASSIGNMENT_BYTES {
            return Err(ProfileAssignmentsError::TooLarge);
        }
        let parent = path
            .parent()
            .ok_or(ProfileAssignmentsError::InsecureDirectory)?;
        validate_profile_directory(parent)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(".applications-{}-{nonce}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
            .map_err(ProfileAssignmentsError::Write)?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(ProfileAssignmentsError::Write)?;
        fs::rename(&temporary, path).map_err(ProfileAssignmentsError::Write)?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(ProfileAssignmentsError::Write)?;
        Ok(())
    }
}

fn validate_profile_name(name: &str) -> Result<(), ProfileAssignmentsError> {
    if name.is_empty()
        || name.len() > 32
        || !name.as_bytes()[0].is_ascii_lowercase()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ProfileAssignmentsError::Invalid(
            "profile names must start with a lowercase letter and contain only lowercase letters, digits, or hyphens (max 32 characters)".into(),
        ));
    }
    Ok(())
}

fn validate_profile_directory(path: &Path) -> Result<(), ProfileAssignmentsError> {
    let metadata = fs::symlink_metadata(path).map_err(ProfileAssignmentsError::Read)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(ProfileAssignmentsError::InsecureDirectory);
    }
    Ok(())
}
