use std::path::Path;

use anyhow::Result;
use wayexpand_core::{import_espanso, Config};

/// Load and validate an Espanso library for review without mutating the
/// active configuration. The caller decides when to apply the preview.
pub(crate) fn preview_espanso(path: &Path) -> Result<(Config, usize)> {
    let imported = import_espanso(path)?;
    Ok((imported.config, imported.skipped))
}
