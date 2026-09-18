# Language Support in WayExpand

WayExpand GUI now supports multiple languages with automatic detection and manual selection.

## Supported Languages

- **English** (en) — Default
- **Deutsch** (de) — German

## Using Different Languages

### GUI Language Selector

The easiest way to switch languages is to use the GUI:

1. Open `wayexpand-gui`
2. Click the **🌐 EN/DE** button in the toolbar
3. Select your preferred language
4. The UI updates immediately

### Environment Variable

Set the `LANG` environment variable to use German by default:

```bash
# Use German
export LANG=de_DE.UTF-8
wayexpand-gui

# Use English (default)
export LANG=en_US.UTF-8
wayexpand-gui
```

Or set it permanently in your shell configuration:

```bash
# ~/.bashrc or ~/.zshrc
export LANG=de_DE.UTF-8
```

## Architecture

### Translation System

The language system is implemented in `crates/gui/src/lang.rs`:

```rust
pub enum Language {
    English,
    German,
}

pub struct Strings {
    lang: Language,
}
```

### Adding New Languages

To add a new language (e.g., French):

1. Add the language variant to the `Language` enum:
```rust
pub enum Language {
    English,
    German,
    French,
}
```

2. Update `Language::from_env()` to detect it:
```rust
pub fn from_env() -> Self {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| {
            if lang.starts_with("fr") {
                Some(Language::French)
            } else if lang.starts_with("de") {
                Some(Language::German)
            } else {
                None
            }
        })
        .unwrap_or(Language::English)
}
```

3. Add translations to each method in `Strings`. For example:
```rust
pub fn ready(&self) -> &'static str {
    match self.lang {
        Language::English => "Ready",
        Language::German => "Fertig",
        Language::French => "Prêt",
    }
}
```

4. Update the language selector dialog in `main.rs`:
```rust
if ui.selectable_label(self.language == Language::French, "Français").clicked() {
    self.language = Language::French;
    self.strings.set_language(Language::French);
}
```

## Translation Coverage

### Implemented

The following UI elements have been translated:

- ✅ Toolbar (title, buttons, search placeholder)
- ✅ Sidebar (snippets list, empty states, filters)
- ✅ Editor (form labels, descriptions, tooltips)
- ✅ Dialogs (diagnostics, settings, import, language selector)
- ✅ Status messages (success, error, action confirmations)
- ✅ Buttons and labels (all interactive elements)

### Not Yet Translated

- Hardcoded error messages from the core library (intentionally in English for debugging)
- System messages from Wayland protocol probes
- Daemon status output (from the backend daemons)

These are intentionally left in English as they contain technical diagnostic information.

## German README

A comprehensive German README is available at `README.de.md`.

To link from other documentation:

```markdown
- English: [README.md](README.md)
- Deutsch: [README.de.md](README.de.md)
```

## Testing

To test language switching:

1. Build the GUI:
```bash
cargo build -p wayexpand-gui --release
```

2. Run with English:
```bash
LANG=en_US.UTF-8 ./target/release/wayexpand-gui
```

3. Run with German (via environment):
```bash
LANG=de_DE.UTF-8 ./target/release/wayexpand-gui
```

4. Run and switch in-app:
```bash
./target/release/wayexpand-gui
# Click 🌐 EN/DE button to switch languages
```

## Future Enhancements

- [ ] Add more languages (French, Spanish, Japanese, etc.)
- [ ] Support for right-to-left languages (Arabic, Hebrew)
- [ ] Community translation crowdsourcing
- [ ] Translation memory for consistency

## Contributing Translations

Want to add your language? Please:

1. Open an issue or discussion on GitHub
2. Translate all strings in the `Strings` struct
3. Test with the GUI
4. Submit a PR with the changes

Thank you for helping make WayExpand accessible in your language!
