use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileStamp {
    modified: SystemTime,
    length: u64,
    device: u64,
    inode: u64,
    mode: u32,
    change_time: i64,
    change_time_nsec: i64,
}

/// A metadata-only stamp for the primary config. This is sampled on the
/// keyboard reactor, so it must never read the file's contents. The actual
/// load and parse happen on the reload worker, which compares the stamp before
/// and after reading to reject concurrent edits.
pub(super) fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let modified = metadata.modified().ok()?;
    let length = metadata.len();
    let device = metadata.dev();
    let inode = metadata.ino();
    let mode = metadata.mode();
    let change_time = metadata.ctime();
    let change_time_nsec = metadata.ctime_nsec();
    Some(FileStamp {
        modified,
        length,
        device,
        inode,
        mode,
        change_time,
        change_time_nsec,
    })
}

/// Bounded content read used only by reload workers, never by the input
/// reactor. It mirrors the config loader's regular-file and size limits and
/// rechecks the opened descriptor in case the path changes between metadata
/// lookup and open.
pub(super) fn read_regular_config_file(path: &Path) -> Option<(fs::Metadata, Vec<u8>)> {
    let preliminary = fs::metadata(path).ok()?;
    if !preliminary.file_type().is_file() || preliminary.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let file = fs::File::from(descriptor);
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
        return None;
    }
    let mut contents = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut contents)
        .ok()?;
    (contents.len() <= MAX_CONFIG_BYTES).then_some((metadata, contents))
}

pub(super) fn file_content_fingerprint(path: &Path) -> Option<u64> {
    let (_, contents) = read_regular_config_file(path)?;
    Some(stable_content_fingerprint(&contents))
}

pub(super) fn stable_content_fingerprint(contents: &[u8]) -> u64 {
    // FNV-1a is deterministic across compiler versions and sensitive to byte
    // order, unlike a commutative chunk sum.
    contents.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

/// The paths whose changes can alter the loaded configuration. The watcher
/// covers whole directories (the configuration's parent, to see atomic
/// replacement, and the fleet layers), and anything else written there --
/// usage statistics, preferences, logs -- must not trigger a reload: every
/// reload rebuilds the engine, which drops the typing buffer and discards a
/// command that is still running.
pub(super) struct WatchFilter {
    /// The configuration file as each watched directory reports it.
    pub(super) files: Vec<PathBuf>,
    /// Directories whose whole contents are configuration layers.
    pub(super) trees: Vec<PathBuf>,
}

impl WatchFilter {
    fn concerns(&self, path: &Path) -> bool {
        self.files.iter().any(|file| file == path)
            || self.trees.iter().any(|tree| path.starts_with(tree))
    }
}

/// Whether a watcher notification may mean configuration contents changed.
///
/// Read-only access never does: the reload worker opens the file itself, and
/// an `IN_OPEN` that requested another reload would rebuild the engine
/// forever. Errors and rescan requests can mean an event was lost (for
/// example, the kernel watch queue overflowed), and an event without paths
/// cannot be attributed, so all three still request a rescan.
pub(super) fn event_requests_rescan(
    event: &notify::Result<notify::Event>,
    filter: &WatchFilter,
) -> bool {
    use notify::event::{AccessKind, AccessMode, EventKind};
    let event = match event {
        Err(_) => return true,
        Ok(event) => event,
    };
    match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => {}
        EventKind::Access(_) => return false,
        _ => {}
    }
    event.need_rescan()
        || event.paths.is_empty()
        || event.paths.iter().any(|path| filter.concerns(path))
}

pub(super) fn start_config_watcher(
    path: &Path,
    fleet: bool,
    dirty: Arc<AtomicBool>,
) -> Option<RecommendedWatcher> {
    let mut watched_paths = std::collections::HashSet::new();
    let mut filter = WatchFilter {
        files: vec![path.to_path_buf()],
        trees: Vec::new(),
    };
    let file_name = path.file_name();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    if let Some(file_name) = file_name {
        filter.files.push(parent.join(file_name));
    }
    filter.trees.push(parent.join("snippets.d"));
    watched_paths.insert(parent);
    if let Ok(resolved) = fs::canonicalize(path) {
        if let Some(parent) = resolved.parent() {
            filter.trees.push(parent.join("snippets.d"));
            watched_paths.insert(parent.to_path_buf());
        }
        filter.files.push(resolved);
    }
    let fleet_directories = if fleet {
        FleetConfig::standard_source_directories()
    } else {
        Vec::new()
    };
    filter.trees.extend(fleet_directories.iter().cloned());

    let callback_dirty = Arc::clone(&dirty);
    let mut watcher = match notify::recommended_watcher(
        move |event: notify::Result<notify::Event>| {
            if event_requests_rescan(&event, &filter) {
                callback_dirty.store(true, Ordering::Release);
            }
        },
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            warn!(%error, "configuration filesystem watcher unavailable; using integrity polling");
            return None;
        }
    };

    let mut watched_config = false;
    for directory in watched_paths {
        match watcher.watch(&directory, RecursiveMode::NonRecursive) {
            Ok(()) => watched_config = true,
            Err(error) => {
                warn!(%error, path = %directory.display(), "could not watch configuration directory")
            }
        }
    }
    if !watched_config {
        warn!("configuration file watch unavailable; using integrity polling");
        return None;
    }

    for directory in fleet_directories {
        if !directory.is_dir() {
            continue;
        }
        if let Err(error) = watcher.watch(&directory, RecursiveMode::Recursive) {
            warn!(%error, path = %directory.display(), "could not watch fleet configuration directory; integrity polling remains enabled");
        }
    }
    Some(watcher)
}
