//! Local XKB dead-key and Compose-sequence tracking for input-method-v2.

use std::{cell::RefCell, ffi::OsStr};
use xkbcommon::xkb::{
    compose::{self, Status},
    Context, Keysym,
};

const MAX_COMPOSE_FALLBACK_BYTES: usize = 256;
const MAX_COMPOSE_SEQUENCE_KEYS: usize = 64;

struct ComposeResources {
    _context: Context,
    _table: compose::Table,
    state: compose::State,
}

impl ComposeResources {
    fn new() -> Option<Self> {
        let context = Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let locale = std::env::var_os("LC_ALL")
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var_os("LC_CTYPE").filter(|value| !value.is_empty()))
            .or_else(|| std::env::var_os("LANG").filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "C.UTF-8".into());
        let table = compose::Table::new_from_locale(
            &context,
            OsStr::new(&locale),
            compose::COMPILE_NO_FLAGS,
        )
        .ok()?;
        let state = compose::State::new(&table, compose::STATE_NO_FLAGS);
        Some(Self {
            _context: context,
            _table: table,
            state,
        })
    }
}

thread_local! {
    // libxkbcommon compose objects are deliberately !Send/!Sync. Keep those
    // handles on the thread that calls the C API; LocalCompose stores only a
    // bounded keysym transcript so InputMethodSource remains movable between
    // the daemon's lifecycle threads.
    static COMPOSE_RESOURCES: RefCell<Option<ComposeResources>> = const { RefCell::new(None) };
}

pub(super) enum ComposeUpdate {
    Inactive,
    Pending,
    Composed(String),
    Cancelled(String),
}

/// Tracks Compose sequences independently from xkb key state. xkb keyboard
/// state resolves layouts and modifiers; libxkbcommon's Compose state knows
/// when a multi-key sequence has actually committed or been cancelled.
pub(super) struct LocalCompose {
    sequence: Vec<u32>,
    fallback: String,
}

impl LocalCompose {
    pub(super) fn new() -> Option<Self> {
        let available = COMPOSE_RESOURCES.with(|resources| {
            let mut resources = resources.borrow_mut();
            if resources.is_none() {
                *resources = ComposeResources::new();
            }
            resources.is_some()
        });
        available.then(|| Self {
            sequence: Vec::new(),
            fallback: String::new(),
        })
    }

    pub(super) fn is_composing(&self) -> bool {
        !self.sequence.is_empty()
    }

    pub(super) fn cancel(&mut self) -> String {
        self.sequence.clear();
        std::mem::take(&mut self.fallback)
    }

    /// Feed the key symbol selected by the active XKB layout. Text from an
    /// incomplete sequence is retained only as a bounded fallback in case
    /// the Compose sequence is cancelled; it is discarded on successful
    /// composition.
    pub(super) fn feed(&mut self, raw_keysym: u32, fallback_text: Option<&str>) -> ComposeUpdate {
        let was_composing = self.is_composing();
        let (status, composed) = match COMPOSE_RESOURCES.try_with(|resources| {
            let mut resources = resources.borrow_mut();
            if resources.is_none() {
                *resources = ComposeResources::new();
            }
            let resources = resources.as_mut()?;
            resources.state.reset();
            for keysym in self
                .sequence
                .iter()
                .copied()
                .chain(std::iter::once(raw_keysym))
            {
                resources.state.feed(Keysym::new(keysym));
            }
            let status = resources.state.status();
            let composed = (status == Status::Composed)
                .then(|| resources.state.utf8())
                .flatten();
            Some((status, composed))
        }) {
            Ok(Some(result)) => result,
            _ => return ComposeUpdate::Inactive,
        };

        match status {
            Status::Composing => {
                if self.sequence.len() >= MAX_COMPOSE_SEQUENCE_KEYS {
                    if let Some(text) = fallback_text {
                        self.fallback.push_str(text);
                    }
                    let fallback = std::mem::take(&mut self.fallback);
                    self.sequence.clear();
                    return ComposeUpdate::Cancelled(fallback);
                }
                self.sequence.push(raw_keysym);
                if let Some(text) = fallback_text {
                    if self.fallback.len().saturating_add(text.len()) <= MAX_COMPOSE_FALLBACK_BYTES
                    {
                        self.fallback.push_str(text);
                    } else {
                        let mut fallback = std::mem::take(&mut self.fallback);
                        fallback.push_str(text);
                        self.sequence.clear();
                        return ComposeUpdate::Cancelled(fallback);
                    }
                }
                ComposeUpdate::Pending
            }
            Status::Composed => {
                let text = composed.unwrap_or_default();
                self.reset();
                if text.is_empty() {
                    ComposeUpdate::Cancelled(fallback_text.unwrap_or_default().to_owned())
                } else {
                    ComposeUpdate::Composed(text)
                }
            }
            Status::Cancelled => {
                if let Some(text) = fallback_text {
                    if self.fallback.len().saturating_add(text.len()) <= MAX_COMPOSE_FALLBACK_BYTES
                    {
                        self.fallback.push_str(text);
                    }
                }
                let fallback = std::mem::take(&mut self.fallback);
                self.reset();
                ComposeUpdate::Cancelled(fallback)
            }
            Status::Nothing if was_composing => {
                if let Some(text) = fallback_text {
                    if self.fallback.len().saturating_add(text.len()) <= MAX_COMPOSE_FALLBACK_BYTES
                    {
                        self.fallback.push_str(text);
                    }
                }
                let fallback = std::mem::take(&mut self.fallback);
                self.reset();
                ComposeUpdate::Cancelled(fallback)
            }
            Status::Nothing => ComposeUpdate::Inactive,
        }
    }

    fn reset(&mut self) {
        self.sequence.clear();
        self.fallback.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposeUpdate, LocalCompose};

    #[test]
    fn multi_key_sequence_stays_pending_until_composed_text_exists() {
        let Some(mut compose) = LocalCompose::new() else {
            panic!("the test locale must provide an XKB Compose table");
        };

        assert!(matches!(
            compose.feed(xkeysym::key::Multi_key, None),
            ComposeUpdate::Pending
        ));
        assert!(matches!(
            compose.feed(xkeysym::key::o, Some("o")),
            ComposeUpdate::Pending
        ));
        assert!(matches!(
            compose.feed(xkeysym::key::c, Some("c")),
            ComposeUpdate::Composed(text) if text == "©"
        ));
    }

    #[test]
    fn dead_key_sequence_commits_the_composed_scalar() {
        let Some(mut compose) = LocalCompose::new() else {
            panic!("the test locale must provide an XKB Compose table");
        };

        assert!(matches!(
            compose.feed(xkeysym::key::dead_acute, None),
            ComposeUpdate::Pending
        ));
        assert!(matches!(
            compose.feed(xkeysym::key::e, Some("e")),
            ComposeUpdate::Composed(text) if text == "é"
        ));
    }

    #[test]
    fn cancelled_sequence_returns_bounded_literal_fallback() {
        let Some(mut compose) = LocalCompose::new() else {
            panic!("the test locale must provide an XKB Compose table");
        };

        let _ = compose.feed(xkeysym::key::Multi_key, None);
        let _ = compose.feed(xkeysym::key::o, Some("o"));
        assert!(matches!(
            compose.feed(xkeysym::key::q, Some("q")),
            ComposeUpdate::Cancelled(text) if text == "oq"
        ));
    }
}
