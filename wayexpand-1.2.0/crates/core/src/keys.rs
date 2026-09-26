//! Backend-independent keyboard chord types.
//!
//! Backends report physical/compositor-specific events; the automation layer
//! consumes this normalized representation. Keeping parsing here prevents
//! each backend and frontend from inventing slightly different hotkey syntax.

use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub modifiers: Modifiers,
    pub key: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyChordError {
    #[error("key chord is empty")]
    Empty,
    #[error("key chord contains an empty component")]
    EmptyComponent,
    #[error("unknown modifier {0:?}")]
    UnknownModifier(String),
    #[error("key chord must contain exactly one key")]
    MultipleKeys,
    #[error("key name is too long")]
    KeyTooLong,
}

impl KeyChord {
    /// Parse a human-readable chord such as `Ctrl+Alt+M` or `Super+Enter`.
    /// Modifier names are case-insensitive; the key name is normalized to
    /// uppercase for stable comparisons and diagnostics.
    pub fn parse(value: &str) -> Result<Self, KeyChordError> {
        if value.trim().is_empty() {
            return Err(KeyChordError::Empty);
        }
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for component in value.split('+') {
            let component = component.trim();
            if component.is_empty() {
                return Err(KeyChordError::EmptyComponent);
            }
            match component.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "option" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "super" | "meta" | "win" | "logo" => modifiers.super_key = true,
                _ if key.is_none() => {
                    if component.chars().count() > 64 {
                        return Err(KeyChordError::KeyTooLong);
                    }
                    key = Some(component.to_ascii_uppercase());
                }
                _ => return Err(KeyChordError::MultipleKeys),
            }
        }
        let Some(key) = key else {
            return Err(KeyChordError::Empty);
        };
        Ok(Self { modifiers, key })
    }

    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let modifiers = [
            (self.modifiers.ctrl, "Ctrl"),
            (self.modifiers.alt, "Alt"),
            (self.modifiers.shift, "Shift"),
            (self.modifiers.super_key, "Super"),
        ];
        let mut first = true;
        for (enabled, name) in modifiers {
            if enabled {
                if !first {
                    formatter.write_str("+")?;
                }
                formatter.write_str(name)?;
                first = false;
            }
        }
        if !first {
            formatter.write_str("+")?;
        }
        formatter.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_chords() {
        let chord = KeyChord::parse("ctrl + alt + m").unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Alt+M");
        assert!(chord.modifiers.ctrl);
        assert!(chord.modifiers.alt);
        assert!(!chord.modifiers.shift);
    }

    #[test]
    fn accepts_common_modifier_aliases() {
        let chord = KeyChord::parse("Control+Option+Win+Enter").unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Alt+Super+ENTER");
    }

    #[test]
    fn rejects_ambiguous_or_empty_chords() {
        assert_eq!(KeyChord::parse("").unwrap_err(), KeyChordError::Empty);
        assert_eq!(
            KeyChord::parse("Ctrl+").unwrap_err(),
            KeyChordError::EmptyComponent
        );
        assert_eq!(
            KeyChord::parse("Ctrl+A+B").unwrap_err(),
            KeyChordError::MultipleKeys
        );
        assert_eq!(KeyChord::parse("Shift").unwrap_err(), KeyChordError::Empty);
    }

    #[test]
    fn equality_is_suitable_for_dispatch() {
        let left = KeyChord::parse("Super+K").unwrap();
        let right = KeyChord::parse("logo+k").unwrap();
        assert!(left.matches(&right));
    }
}
