//! Whether the daemon's current output route can type each snippet, shown as
//! a badge in the snippet list and a note under the editor preview. Both use
//! core's `route_limit`, the check the daemon makes before typing, so the GUI
//! never promises more than the route will do.

use crate::theme::RouteBadge;
use crate::GuiApp;
use wayexpand_core::{route_limit, ExpansionEngine, InjectorCapabilities, RouteFit, RouteLimit};

impl GuiApp {
    /// One verdict per saved snippet for the route the daemon reports now, or
    /// `None` while that route is unknown. Recomputed only when the saved
    /// configuration or the route changes.
    pub(crate) fn route_fits(&mut self) -> Option<&[RouteFit]> {
        let capabilities = self.daemon_capabilities.as_ref()?.output_capabilities()?;
        let stale = !matches!(
            &self.route_fit_cache,
            Some((revision, cached, _)) if *revision == self.config_revision && *cached == capabilities
        );
        if stale {
            let engine = ExpansionEngine::new(self.config.clone()).ok()?;
            let fits = (0..self.config.expansion.len())
                .map(|index| engine.route_fit(index, &capabilities))
                .collect();
            self.route_fit_cache = Some((self.config_revision.clone(), capabilities, fits));
        }
        self.route_fit_cache
            .as_ref()
            .map(|(_, _, fits)| fits.as_slice())
    }

    pub(crate) fn route_badge(&self, fit: RouteFit) -> Option<RouteBadge<'static>> {
        let (label, warning) = match fit {
            RouteFit::Types | RouteFit::CheckedWhenUsed => return None,
            RouteFit::DependsOnLayout => (self.strings.route_badge_layout(), false),
            RouteFit::Refused(RouteLimit::TooLong { .. }) => {
                (self.strings.route_badge_too_long(), true)
            }
            RouteFit::Refused(RouteLimit::Unicode { .. }) => {
                (self.strings.route_badge_cannot_type(), true)
            }
            RouteFit::Refused(RouteLimit::CursorMove) => (self.strings.route_badge_cursor(), true),
        };
        Some(RouteBadge { label, warning })
    }

    pub(crate) fn route_fit_hint(&self, fit: RouteFit) -> Option<String> {
        match fit {
            RouteFit::Types | RouteFit::CheckedWhenUsed => None,
            RouteFit::DependsOnLayout => Some(self.strings.route_fit_layout_hint().to_owned()),
            RouteFit::Refused(RouteLimit::TooLong { chars, max }) => {
                Some(self.strings.route_fit_too_long_hint(chars, max))
            }
            RouteFit::Refused(RouteLimit::Unicode { position, .. }) => {
                Some(self.strings.route_fit_cannot_type_hint(position))
            }
            RouteFit::Refused(RouteLimit::CursorMove) => {
                Some(self.strings.route_fit_cursor_hint().to_owned())
            }
        }
    }
}

/// The verdict for text already rendered by the editor preview, using the
/// same rule as [`ExpansionEngine::route_fit`].
pub(crate) fn rendered_route_fit(
    capabilities: &InjectorCapabilities,
    rendered: &str,
    reinsert_after: Option<char>,
    cursor_offset: Option<usize>,
) -> RouteFit {
    match route_limit(capabilities, rendered, reinsert_after, cursor_offset) {
        Some(limit) => RouteFit::Refused(limit),
        None if !capabilities.full_unicode
            && capabilities.layout_checked_text
            && !rendered.is_ascii() =>
        {
            RouteFit::DependsOnLayout
        }
        None => RouteFit::Types,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keysym(layout_checked_text: bool) -> InjectorCapabilities {
        InjectorCapabilities {
            max_text_chars: 8,
            layout_checked_text,
            ..InjectorCapabilities::default()
        }
    }

    #[test]
    fn rendered_text_gets_the_daemons_verdict() {
        assert_eq!(
            rendered_route_fit(&keysym(false), "Thanks", None, None),
            RouteFit::Types
        );
        assert_eq!(
            rendered_route_fit(&keysym(true), "Grüße", None, None),
            RouteFit::DependsOnLayout
        );
        assert!(matches!(
            rendered_route_fit(&keysym(false), "Grüße", None, None),
            RouteFit::Refused(RouteLimit::Unicode { position: 3, .. })
        ));
        assert!(matches!(
            rendered_route_fit(&keysym(true), "Best regards, Ada", None, None),
            RouteFit::Refused(RouteLimit::TooLong { chars: 17, max: 8 })
        ));
        let full = InjectorCapabilities {
            full_unicode: true,
            ..InjectorCapabilities::default()
        };
        assert_eq!(
            rendered_route_fit(&full, "東京 café", None, None),
            RouteFit::Types
        );
    }
}
