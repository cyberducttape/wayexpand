//! The application's single status line.
//!
//! Status text is produced in a dozen different places (saves, imports,
//! daemon control, window detection) and every one of them knows whether
//! what just happened was a success, a partial success, or a failure. That
//! intent is recorded here explicitly instead of being recovered afterwards
//! by matching English keywords in the rendered sentence -- which silently
//! stopped working for any translated or reworded message and coloured a
//! failure as neutral.

use eframe::egui::Color32;

use crate::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StatusTone {
    /// Neutral state, no outcome to report (the idle "Ready" line).
    Info,
    /// The requested operation completed.
    Success,
    /// The operation completed, but something the user should know about
    /// went wrong alongside it (a save that the daemon then refused to
    /// reload), or the request was a no-op.
    Warning,
    /// The operation did not happen.
    Error,
}

/// One status line: what to say, and how it should read.
#[derive(Clone, Debug)]
pub(crate) struct Status {
    tone: StatusTone,
    text: String,
}

impl Status {
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Self {
            tone: StatusTone::Info,
            text: text.into(),
        }
    }

    pub(crate) fn success(text: impl Into<String>) -> Self {
        Self {
            tone: StatusTone::Success,
            text: text.into(),
        }
    }

    pub(crate) fn warning(text: impl Into<String>) -> Self {
        Self {
            tone: StatusTone::Warning,
            text: text.into(),
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            tone: StatusTone::Error,
            text: text.into(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The recorded tone. Only the renderer needs the colours, so this exists
    /// for tests that assert an outcome was classified correctly rather than
    /// asserting on its wording.
    #[cfg(test)]
    pub(crate) fn tone_for_test(&self) -> StatusTone {
        self.tone
    }

    /// Appends a caveat to an otherwise successful outcome. A success is
    /// downgraded to a warning so the line is not read as "everything
    /// worked"; an existing warning or error keeps its stronger tone.
    pub(crate) fn with_caveat(mut self, caveat: impl AsRef<str>) -> Self {
        self.text = format!("{} ({})", self.text, caveat.as_ref());
        if self.tone == StatusTone::Success || self.tone == StatusTone::Info {
            self.tone = StatusTone::Warning;
        }
        self
    }

    /// Foreground text colour and the colour of the accent bar that marks
    /// the tone at the start of the status line. `Info` draws no bar.
    pub(crate) fn colors(&self, palette: &Palette) -> (Color32, Option<Color32>) {
        match self.tone {
            StatusTone::Info => (palette.muted, None),
            StatusTone::Success => (palette.success, Some(palette.success)),
            StatusTone::Warning => (palette.warning, Some(palette.warning)),
            StatusTone::Error => (palette.danger, Some(palette.danger)),
        }
    }

    /// A faint wash of the tone colour behind the line, so an error is
    /// noticeable without the status bar shouting during normal use.
    ///
    /// Always opaque: nothing is painted beneath the status panel, so a
    /// transparent fill showed the window's clear colour -- a near-black bar
    /// in the light theme with the muted status text unreadable on it.
    pub(crate) fn background(&self, palette: &Palette) -> Color32 {
        let wash = match self.tone {
            StatusTone::Info => return palette.surface,
            StatusTone::Success => theme::tint(palette.success, 20),
            StatusTone::Warning => theme::tint(palette.warning, 24),
            StatusTone::Error => theme::tint(palette.danger, 26),
        };
        theme::blend_over(wash, palette.surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colorpack::ColorPack;

    #[test]
    fn every_status_tone_paints_an_opaque_readable_background() {
        for dark in [false, true] {
            let palette = Palette::for_pack(ColorPack::Default, dark);
            for status in [
                Status::info("Ready"),
                Status::success("Saved"),
                Status::warning("Careful"),
                Status::error("Failed"),
            ] {
                let background = status.background(&palette);
                assert_eq!(background.a(), 255, "{:?} dark={dark}", status.tone);
                let (text, _) = status.colors(&palette);
                assert!(
                    crate::colorpack::contrast_ratio(text, background)
                        >= crate::colorpack::WCAG_AA_NORMAL_TEXT,
                    "{:?} dark={dark}",
                    status.tone
                );
            }
        }
    }

    #[test]
    fn a_caveat_downgrades_a_success_to_a_warning() {
        let status = Status::success("Snippet saved").with_caveat("daemon did not reload");
        assert_eq!(status.tone, StatusTone::Warning);
        assert_eq!(status.text(), "Snippet saved (daemon did not reload)");
    }

    #[test]
    fn a_caveat_never_softens_a_failure() {
        let status = Status::error("Save failed").with_caveat("disk full");
        assert_eq!(status.tone, StatusTone::Error);
    }

    #[test]
    fn tones_are_coloured_independently_of_their_wording() {
        let palette = Palette::for_pack(ColorPack::Default, true);
        // Deliberately worded so the old English keyword heuristic would
        // have read it as a success ("saved") rather than a failure.
        let status = Status::error("Nicht gespeichert: saved nothing");
        let (foreground, bar) = status.colors(&palette);
        assert_eq!(foreground, palette.danger);
        assert_eq!(bar, Some(palette.danger));
    }

    #[test]
    fn the_idle_line_stays_unobtrusive() {
        let palette = Palette::for_pack(ColorPack::Default, true);
        let status = Status::info("Ready");
        assert_eq!(status.colors(&palette), (palette.muted, None));
        // No tone wash: the idle line is the plain surface colour.
        assert_eq!(status.background(&palette), palette.surface);
    }
}
