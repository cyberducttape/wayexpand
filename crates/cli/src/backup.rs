//! Configuration backup: default destination and archive creation.

use crate::*;
use std::io::{Read, Seek, SeekFrom};

/// `<config>.bak`, or the first free `<config>.bak.N`, so running `backup`
/// again keeps every earlier backup instead of failing on the first one.
/// `create_backup` still refuses to overwrite if a name appears meanwhile.
pub(crate) fn default_backup_destination(source: &Path) -> PathBuf {
    let mut base = source.as_os_str().to_owned();
    base.push(".bak");
    let first = PathBuf::from(&base);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2..10_000)
        .map(|index| {
            let mut candidate = base.clone();
            candidate.push(format!(".{index}"));
            PathBuf::from(candidate)
        })
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .unwrap_or(first)
}

pub(crate) fn create_backup(source: &Path, destination: &Path) -> Result<()> {
    create_backup_with_sync(source, destination, |directory| directory.sync_all())
}

fn create_backup_with_sync(
    source: &Path,
    destination: &Path,
    sync_directory: impl FnOnce(&fs::File) -> io::Result<()>,
) -> Result<()> {
    create_backup_with_hooks(source, destination, || {}, sync_directory)
}

fn create_backup_with_hooks(
    source: &Path,
    destination: &Path,
    after_copy: impl FnOnce(),
    sync_directory: impl FnOnce(&fs::File) -> io::Result<()>,
) -> Result<()> {
    // Inspect the opened file, not the path, so the size checked below
    // belongs to the same file that is copied. O_NONBLOCK keeps a FIFO from
    // blocking the open before the regular-file check can reject it.
    let mut source_file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(source)
        .with_context(|| format!("opening configuration {}", source.display()))?;
    let metadata = source_file
        .metadata()
        .with_context(|| format!("reading configuration {}", source.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", source.display());
    }

    let mut destination_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .with_context(|| format!("creating configuration backup {}", destination.display()))?;
    // A backup is only reported after its file and parent directory are
    // synced. Failures before the directory sync remove incomplete file data;
    // a failure at the final sync leaves the complete backup in place and
    // reports its durability as uncertain below.
    let mut after_copy = Some(after_copy);
    let written = io::copy(&mut source_file, &mut destination_file).and_then(|copied| {
        if copied != metadata.len() {
            return Err(io::Error::other(format!(
                "copied {copied} of {} bytes; the configuration changed during backup",
                metadata.len()
            )));
        }
        after_copy
            .take()
            .expect("backup copy hook is called once")();
        if !file_contents_match(&mut source_file, &mut destination_file, metadata.len())? {
            return Err(io::Error::other(
                "configuration changed during backup (copied content differs); refusing to keep a possibly inconsistent copy",
            ));
        }
        let final_metadata = source_file.metadata()?;
        if source_changed_during_backup(&metadata, &final_metadata)
            || !source_path_matches_open_file(source, &metadata)?
        {
            return Err(io::Error::other(
                "configuration changed during backup; refusing to keep a possibly inconsistent copy",
            ));
        }
        destination_file.sync_all()
    });
    if let Err(error) = written {
        drop(destination_file);
        let _ = fs::remove_file(destination);
        return Err(error)
            .with_context(|| format!("writing configuration backup {}", destination.display()));
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Err(error) = fs::File::open(parent).and_then(|directory| sync_directory(&directory)) {
        bail!(
            "configuration backup {} was created, but syncing its parent directory failed; durability is uncertain: {error}",
            destination.display()
        );
    }
    Ok(())
}

/// Compare the copied backup with the still-open source without relying on
/// filesystem timestamp resolution to detect same-size in-place edits.
fn file_contents_match(
    source: &mut fs::File,
    backup: &mut fs::File,
    expected_len: u64,
) -> io::Result<bool> {
    source.seek(SeekFrom::Start(0))?;
    backup.seek(SeekFrom::Start(0))?;

    let mut source_buffer = [0_u8; 16 * 1024];
    let mut backup_buffer = [0_u8; 16 * 1024];
    let mut remaining = expected_len;
    while remaining > 0 {
        let chunk_len = remaining.min(source_buffer.len() as u64) as usize;
        source.read_exact(&mut source_buffer[..chunk_len])?;
        backup.read_exact(&mut backup_buffer[..chunk_len])?;
        if source_buffer[..chunk_len] != backup_buffer[..chunk_len] {
            return Ok(false);
        }
        remaining -= chunk_len as u64;
    }

    let source_extra = source.read(&mut source_buffer[..1])?;
    let backup_extra = backup.read(&mut backup_buffer[..1])?;
    Ok(source_extra == backup_extra
        && source_buffer[..source_extra] == backup_buffer[..backup_extra])
}

fn source_changed_during_backup(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
}

/// A stable descriptor alone is not enough: an atomic rename can replace the
/// configured path while the descriptor continues to refer to the old file.
/// Confirm the pathname still identifies the original regular file before
/// declaring the backup complete.
fn source_path_matches_open_file(path: &Path, opened: &fs::Metadata) -> io::Result<bool> {
    let current = fs::symlink_metadata(path)?;
    Ok(current.is_file() && current.dev() == opened.dev() && current.ino() == opened.ino())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn directory_sync_failure_reports_that_backup_was_created() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-backup-durability-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.toml");
        let destination = directory.join("source.toml.bak");
        fs::write(&source, b"configuration").unwrap();

        let error = create_backup_with_sync(&source, &destination, |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("backup"));
        assert!(message.contains("durability is uncertain"));
        assert_eq!(fs::read(&destination).unwrap(), b"configuration");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn same_size_source_modification_discards_the_backup() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-backup-source-change-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.toml");
        let destination = directory.join("source.toml.bak");
        fs::write(&source, b"first-config").unwrap();

        let error = create_backup_with_hooks(
            &source,
            &destination,
            || fs::write(&source, b"other-config").unwrap(),
            |directory| directory.sync_all(),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("configuration changed during backup"));
        assert!(!destination.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomic_source_replacement_discards_a_backup_of_the_old_file() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-backup-source-rename-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.toml");
        let replacement = directory.join("replacement.toml");
        let destination = directory.join("source.toml.bak");
        fs::write(&source, b"first-config").unwrap();
        fs::write(&replacement, b"other-config").unwrap();

        let error = create_backup_with_hooks(
            &source,
            &destination,
            || fs::rename(&replacement, &source).unwrap(),
            |directory| directory.sync_all(),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("configuration changed during backup"));
        assert!(!destination.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_source_is_rejected_without_creating_a_backup() {
        use std::os::unix::fs::symlink;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wayexpand-backup-source-symlink-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let target = directory.join("target.toml");
        let source = directory.join("source.toml");
        let destination = directory.join("source.toml.bak");
        fs::write(&target, b"configuration").unwrap();
        symlink(&target, &source).unwrap();

        assert!(create_backup(&source, &destination).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"configuration");
        assert!(!destination.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
