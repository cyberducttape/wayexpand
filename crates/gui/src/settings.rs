use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use serde::{Deserialize, Serialize};

use wayexpand_core::default_config_path;

use crate::{colorpack::ColorPack, lang::Language};

/// GUI-only display preferences. This file is deliberately separate from the
/// expansion configuration so a preferences parse failure never blocks editing.
pub(crate) struct GuiPrefs {
    pub(crate) language: Language,
    pub(crate) colorpack: ColorPack,
    /// `None` means no preference has been saved; the caller should use the
    /// desktop theme on first launch.
    pub(crate) dark_mode: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredGuiPrefs {
    language: String,
    colorpack: String,
    #[serde(default)]
    dark_mode: Option<bool>,
}

fn gui_prefs_path() -> PathBuf {
    default_config_path().with_file_name("gui-prefs.toml")
}

pub(crate) fn load_gui_prefs() -> GuiPrefs {
    let defaults = GuiPrefs {
        language: Language::from_env(),
        colorpack: ColorPack::Default,
        dark_mode: None,
    };
    let Ok(contents) = fs::read_to_string(gui_prefs_path()) else {
        return defaults;
    };
    let Ok(stored) = toml_edit::de::from_str::<StoredGuiPrefs>(&contents) else {
        return defaults;
    };
    let (Some(language), Some(colorpack)) = (
        Language::from_code(&stored.language),
        ColorPack::from_code(&stored.colorpack),
    ) else {
        return defaults;
    };
    GuiPrefs {
        language,
        colorpack,
        dark_mode: stored.dark_mode,
    }
}

/// Saves display preferences atomically. A failure is returned to the caller
/// so the GUI can keep the new appearance while explaining that it will not
/// survive the next launch.
pub(crate) fn save_gui_prefs(
    language: Language,
    colorpack: ColorPack,
    dark_mode: bool,
) -> Result<(), String> {
    let path = gui_prefs_path();
    let parent = path
        .parent()
        .ok_or_else(|| "preferences path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create preferences directory: {error}"))?;
    let stored = StoredGuiPrefs {
        language: language.code().to_owned(),
        colorpack: colorpack.code().to_owned(),
        dark_mode: Some(dark_mode),
    };
    let contents = toml_edit::ser::to_string_pretty(&stored)
        .map_err(|error| format!("could not serialize preferences: {error}"))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("gui-prefs.toml"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|error| format!("could not create temporary preferences file: {error}"))?;
    let result = (|| {
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("could not write preferences: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("could not flush preferences: {error}"))?;
        fs::rename(&temp, &path).map_err(|error| format!("could not replace preferences: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_boolean_is_rejected_instead_of_coerced() {
        let parsed = toml_edit::de::from_str::<StoredGuiPrefs>(
            "language = \"en\"\ncolorpack = \"default\"\ndark_mode = \"maybe\"\n",
        );
        assert!(parsed.is_err());
    }
}
