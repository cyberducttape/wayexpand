//! Configuration backup: default destination and archive creation.

use crate::*;

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
    let metadata = fs::metadata(source)
        .with_context(|| format!("reading configuration {}", source.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", source.display());
    }

    let mut source_file = fs::File::open(source)
        .with_context(|| format!("opening configuration {}", source.display()))?;
    let mut destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .with_context(|| format!("creating configuration backup {}", destination.display()))?;
    if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
        let _ = fs::remove_file(destination);
        return Err(error)
            .with_context(|| format!("writing configuration backup {}", destination.display()));
    }
    Ok(())
}
