use std::{fs, path::PathBuf};

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

fn gui_prefs_path() -> PathBuf {
    default_config_path().with_file_name("gui-prefs.toml")
}

pub(crate) fn load_gui_prefs() -> GuiPrefs {
    let mut prefs = GuiPrefs {
        language: Language::from_env(),
        colorpack: ColorPack::Default,
        dark_mode: None,
    };
    let Ok(contents) = fs::read_to_string(gui_prefs_path()) else {
        return prefs;
    };
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "language" => {
                if let Some(language) = Language::from_code(value) {
                    prefs.language = language;
                }
            }
            "colorpack" => {
                if let Some(colorpack) = ColorPack::from_code(value) {
                    prefs.colorpack = colorpack;
                }
            }
            "dark_mode" => prefs.dark_mode = Some(value == "true"),
            _ => {}
        }
    }
    prefs
}

/// Best-effort save: display preferences are not load-bearing.
pub(crate) fn save_gui_prefs(language: Language, colorpack: ColorPack, dark_mode: bool) {
    let path = gui_prefs_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let contents = format!(
        "language = \"{}\"\ncolorpack = \"{}\"\ndark_mode = {}\n",
        language.code(),
        colorpack.code(),
        dark_mode
    );
    let _ = fs::write(path, contents);
}
