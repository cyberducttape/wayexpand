use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use serde::{Deserialize, Serialize};

use wayexpand_core::default_config_path;

use crate::{colorpack::ColorPack, lang::Language};
use wayexpand_core::FontScale;

/// GUI-only display preferences. This file is deliberately separate from the
/// expansion configuration so a preferences parse failure never blocks editing.
pub(crate) struct GuiPrefs {
    pub(crate) language: Language,
    pub(crate) colorpack: ColorPack,
    pub(crate) font_scale: FontScale,
    /// `None` means no preference has been saved; the caller should use the
    /// desktop theme on first launch.
    pub(crate) dark_mode: Option<bool>,
    /// Whether the user completed or skipped the first-run guide.
    pub(crate) onboarding_completed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredGuiPrefs {
    language: String,
    colorpack: String,
    #[serde(default)]
    font_scale: FontScale,
    #[serde(default)]
    dark_mode: Option<bool>,
    #[serde(default)]
    onboarding_completed: bool,
}

fn gui_prefs_path() -> PathBuf {
    default_config_path().with_file_name("gui-prefs.toml")
}

pub(crate) fn load_gui_prefs() -> GuiPrefs {
    let defaults = GuiPrefs {
        language: Language::from_env(),
        colorpack: ColorPack::Default,
        font_scale: FontScale::Normal,
        dark_mode: None,
        onboarding_completed: false,
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
        font_scale: stored.font_scale,
        dark_mode: stored.dark_mode,
        onboarding_completed: stored.onboarding_completed,
    }
}

/// Saves display preferences atomically. A failure is returned to the caller
/// so the GUI can keep the new appearance while explaining that it will not
/// survive the next launch.
pub(crate) fn save_gui_prefs(
    language: Language,
    colorpack: ColorPack,
    font_scale: FontScale,
    dark_mode: bool,
    onboarding_completed: bool,
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
        font_scale,
        dark_mode: Some(dark_mode),
        onboarding_completed,
    };
    let contents = toml_edit::ser::to_string_pretty(&stored)
        .map_err(|error| format!("could not serialize preferences: {error}"))?;
    write_gui_prefs_atomic(&path, &contents, |parent| {
        fs::File::open(parent)?.sync_all()
    })
}

fn write_gui_prefs_atomic(
    path: &std::path::Path,
    contents: &str,
    sync_directory: impl FnOnce(&std::path::Path) -> io::Result<()>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "preferences path has no parent directory".to_owned())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("gui-prefs.toml");
    let mut temporary = None;
    let mut file = None;
    let mut last_error = None;
    for attempt in 0..16 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let candidate = parent.join(format!(
            ".{file_name}.tmp.{}.{}.{}",
            std::process::id(),
            nonce,
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(opened) => {
                temporary = Some(candidate);
                file = Some(opened);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => {
                return Err(format!(
                    "could not create temporary preferences file: {error}"
                ));
            }
        }
    }
    let temporary = temporary.ok_or_else(|| {
        format!(
            "could not allocate a unique temporary preferences file: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "temporary path collision".into())
        )
    })?;
    let mut file = file.expect("temporary preferences path and file are allocated together");
    let prepared = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = prepared {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not write preferences: {error}"));
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not replace preferences: {error}"));
    }
    sync_directory(parent).map_err(|error| {
        format!(
            "preferences were replaced, but syncing their parent directory failed; durability is uncertain and the saved settings should be reloaded before retrying: {error}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wayexpand-gui-prefs-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn malformed_boolean_is_rejected_instead_of_coerced() {
        let parsed = toml_edit::de::from_str::<StoredGuiPrefs>(
            "language = \"en\"\ncolorpack = \"default\"\ndark_mode = \"maybe\"\n",
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn onboarding_completion_is_backward_compatible_and_persistable() {
        let legacy = toml_edit::de::from_str::<StoredGuiPrefs>(
            "language = \"en\"\ncolorpack = \"default\"\n",
        )
        .unwrap();
        assert!(!legacy.onboarding_completed);
        let completed = toml_edit::de::from_str::<StoredGuiPrefs>(
            "language = \"en\"\ncolorpack = \"default\"\nonboarding_completed = true\n",
        )
        .unwrap();
        assert!(completed.onboarding_completed);
    }

    #[test]
    fn stale_process_id_temp_file_does_not_block_preferences_save() {
        let directory = test_directory();
        let path = directory.join("gui-prefs.toml");
        let stale = directory.join(format!(".gui-prefs.toml.tmp-{}", std::process::id()));
        fs::write(&stale, "stale temp").unwrap();

        write_gui_prefs_atomic(&path, "language = \"en\"\n", |parent| {
            fs::File::open(parent)?.sync_all()
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&stale).unwrap(), "stale temp");
        assert_eq!(fs::read_to_string(&path).unwrap(), "language = \"en\"\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_sync_failure_reports_that_preferences_were_replaced() {
        let directory = test_directory();
        let path = directory.join("gui-prefs.toml");

        let error = write_gui_prefs_atomic(&path, "language = \"en\"\n", |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap_err();

        assert!(error.contains("preferences were replaced"));
        assert!(error.contains("durability is uncertain"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "language = \"en\"\n");
        fs::remove_dir_all(directory).unwrap();
    }
}
