//! Side-effect-free explanation of whether typed text would expand, and if
//! not, which check stops it. Runs the same decisions the matcher and
//! dispatcher make, against the engine's current state, without executing
//! commands or changing matcher state. Replacement text is never included,
//! only its size.

use serde::Serialize;

use super::ExpansionEngine;
use crate::{render_template_with_cursor, AppFilter, MatchMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplainCheck {
    pub name: &'static str,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Explanation {
    pub typed: String,
    /// The configured trigger of the snippet the text resolves to, if any.
    pub snippet: Option<String>,
    pub checks: Vec<ExplainCheck>,
}

impl Explanation {
    pub fn push(&mut self, name: &'static str, status: CheckStatus, detail: impl Into<String>) {
        self.checks.push(ExplainCheck {
            name,
            status,
            detail: detail.into(),
        });
    }

    /// The first failing check, or `None` when the text would expand.
    pub fn suppressed_by(&self) -> Option<&ExplainCheck> {
        self.checks
            .iter()
            .find(|check| check.status == CheckStatus::Fail)
    }

    pub fn would_expand(&self) -> bool {
        self.suppressed_by().is_none()
    }

    /// A checklist for terminals: one line per check and a result line.
    pub fn render_text(&self) -> String {
        let mut text = format!("Typed: {}\n\n", self.typed);
        for check in &self.checks {
            let mark = match check.status {
                CheckStatus::Pass => "✓",
                CheckStatus::Fail => "✗",
                CheckStatus::Warn => "!",
                CheckStatus::Info => "·",
            };
            text.push_str(&format!("{mark} {}: {}\n", check.name, check.detail));
        }
        text.push('\n');
        match self.suppressed_by() {
            Some(check) => text.push_str(&format!("Result: suppressed by {}.\n", check.name)),
            None => text.push_str("Result: would expand.\n"),
        }
        text
    }
}

impl ExpansionEngine {
    /// Explain what would happen if `typed` were typed now. `backend` is the
    /// policy identity of the active output route (for example `libei` or
    /// `ibus`), used for the organization backend allowlist.
    pub fn explain(&self, typed: &str, backend: &str) -> Explanation {
        let typed = typed.trim_end().to_owned();
        let mut explanation = Explanation {
            typed: typed.clone(),
            snippet: None,
            checks: Vec::new(),
        };

        // The snippet whose trigger, alias, or variant the text ends with;
        // the longest such trigger wins, as it would for the matcher.
        let candidate = self
            .config
            .expansion
            .iter()
            .enumerate()
            .flat_map(|(index, expansion)| {
                expansion
                    .effective_triggers()
                    .into_iter()
                    .filter(|variant| typed.ends_with(variant.as_str()))
                    .map(move |variant| (index, variant))
            })
            .max_by_key(|(index, variant)| {
                (
                    self.config.expansion[*index].enabled,
                    variant.chars().count(),
                )
            });
        let Some((index, variant)) = candidate else {
            explanation.push("trigger", CheckStatus::Fail, self.no_match_hint(&typed));
            return explanation;
        };
        let expansion = &self.config.expansion[index];
        explanation.snippet = Some(expansion.trigger.clone());
        let how = if variant == expansion.trigger {
            String::new()
        } else if expansion.aliases.contains(&variant) {
            format!(" (alias of {})", expansion.trigger)
        } else {
            format!(
                " (case or Unicode-normalization variant of {})",
                expansion.trigger
            )
        };
        explanation.push(
            "trigger",
            CheckStatus::Pass,
            format!("recognized as {variant}{how}"),
        );

        if expansion.enabled {
            explanation.push("enabled", CheckStatus::Pass, "snippet is enabled");
        } else {
            explanation.push("enabled", CheckStatus::Fail, "snippet is disabled");
        }

        match expansion.match_mode {
            MatchMode::Immediate => {
                let longer = self
                    .config
                    .expansion
                    .iter()
                    .filter(|other| other.enabled)
                    .find_map(|other| {
                        other.effective_triggers().into_iter().find(|trigger| {
                            trigger.len() > variant.len() && trigger.starts_with(variant.as_str())
                        })
                    });
                match longer {
                    Some(longer) => explanation.push(
                        "match mode",
                        CheckStatus::Warn,
                        format!(
                            "immediate, but waits for the next key because {longer} starts with {variant}"
                        ),
                    ),
                    None => explanation.push(
                        "match mode",
                        CheckStatus::Info,
                        "immediate: fires as soon as the last character is typed",
                    ),
                }
            }
            MatchMode::WordBoundary => explanation.push(
                "match mode",
                CheckStatus::Info,
                "word boundary: fires when a space or punctuation follows, and the character \
                 before the trigger must not be a letter or digit",
            ),
        }

        self.explain_app_filter(&mut explanation, index);

        if self.user_paused {
            explanation.push("paused", CheckStatus::Fail, "WayExpand is paused");
        } else {
            explanation.push("paused", CheckStatus::Pass, "not paused");
        }
        if self.sensitive_focus {
            explanation.push(
                "sensitive field",
                CheckStatus::Fail,
                "the focused field is a password or sensitive field, or has not reported \
                 that it is safe",
            );
        } else {
            explanation.push(
                "sensitive field",
                CheckStatus::Pass,
                "not a sensitive field",
            );
        }
        if self.composition_active {
            explanation.push(
                "composition",
                CheckStatus::Fail,
                "an input-method composition (preedit) is active",
            );
        } else {
            explanation.push("composition", CheckStatus::Pass, "no active composition");
        }

        match &expansion.command {
            Some(command) => {
                if self.command_execution_disabled(command) {
                    explanation.push(
                        "command",
                        CheckStatus::Fail,
                        "command-backed, and command execution is disabled here (organization \
                         policy, direct programs on this route, or unavailable workers)",
                    );
                } else if let Some(action) = &command.action {
                    explanation.push(
                        "command",
                        CheckStatus::Info,
                        format!(
                            "runs Action Broker action {action}; the broker must be running and \
                             finish within {} ms",
                            command.timeout_ms
                        ),
                    );
                } else {
                    explanation.push(
                        "command",
                        CheckStatus::Info,
                        format!(
                            "runs {} and must finish within {} ms",
                            command.program, command.timeout_ms
                        ),
                    );
                }
            }
            None => {
                let variables = crate::template_variables(&expansion.replacement);
                if variables.contains(&"clipboard") {
                    explanation.push(
                        "variables",
                        CheckStatus::Info,
                        "reads the clipboard (allowed by settings.allow_clipboard)",
                    );
                }
                let env: Vec<&str> = variables
                    .iter()
                    .filter_map(|name| name.strip_prefix("env:"))
                    .map(str::trim)
                    .collect();
                if !env.is_empty() {
                    explanation.push(
                        "variables",
                        CheckStatus::Info,
                        format!("reads environment variables {}", env.join(", ")),
                    );
                }
                if variables.iter().any(|name| name.starts_with("snippet:")) {
                    explanation.push("variables", CheckStatus::Info, "includes other snippets");
                }
                match render_template_with_cursor(&expansion.replacement, &self.template_context())
                {
                    Ok((rendered, _)) => explanation.push(
                        "replacement",
                        CheckStatus::Pass,
                        format!("renders to {} characters", rendered.chars().count()),
                    ),
                    Err(error) => explanation.push(
                        "replacement",
                        CheckStatus::Fail,
                        format!("template cannot be rendered: {error}"),
                    ),
                }
            }
        }

        let size = expansion.replacement.len();
        match self.config.organization.expansion_policy_violation(
            size,
            expansion.command.is_some(),
            backend,
        ) {
            Some(violation) if self.config.organization.safe_mode => {
                explanation.push("policy", CheckStatus::Fail, violation);
            }
            Some(violation) => explanation.push(
                "policy",
                CheckStatus::Warn,
                format!("{violation} (audit mode: reported, not blocked)"),
            ),
            None => explanation.push(
                "policy",
                CheckStatus::Pass,
                "allowed by organization policy",
            ),
        }
        explanation
    }

    fn explain_app_filter(&self, explanation: &mut Explanation, index: usize) {
        let expansion = &self.config.expansion[index];
        if expansion.app_filter.is_empty() {
            explanation.push(
                "app filter",
                CheckStatus::Pass,
                "applies in every application",
            );
            return;
        }
        let filters = expansion.app_filter.join(", ");
        let Some(window) = &self.normalized_window else {
            explanation.push(
                "app filter",
                CheckStatus::Fail,
                format!(
                    "limited to {filters}, but no focused application is known (window tracking \
                     is unavailable on this route); app-filtered snippets fail closed"
                ),
            );
            return;
        };
        let app = window.app_id.as_deref().unwrap_or("(unknown app ID)");
        let title_disabled = self.config.organization.disable_title_matching
            && self.app_filters[index]
                .iter()
                .any(|filter| matches!(filter, AppFilter::TitleContains(_)));
        if self.app_filter_allows(index, expansion) {
            explanation.push(
                "app filter",
                CheckStatus::Pass,
                format!("active application {app} matches {filters}"),
            );
        } else {
            let mut detail = format!("active application {app} is not allowed by {filters}");
            if title_disabled {
                detail.push_str(" (title filters are disabled by organization policy)");
            }
            explanation.push("app filter", CheckStatus::Fail, detail);
        }
    }

    fn no_match_hint(&self, typed: &str) -> String {
        let lowered = typed.to_lowercase();
        for expansion in &self.config.expansion {
            for trigger in std::iter::once(&expansion.trigger).chain(&expansion.aliases) {
                if lowered.ends_with(&trigger.to_lowercase()) && !expansion.propagate_case {
                    return format!(
                        "no snippet uses this trigger; {trigger} differs only in case (type it \
                         exactly, or enable case propagation on that snippet)"
                    );
                }
            }
        }
        let prefix_of = self.config.expansion.iter().find_map(|expansion| {
            std::iter::once(&expansion.trigger)
                .chain(&expansion.aliases)
                .find(|trigger| trigger.starts_with(typed) && trigger.as_str() != typed)
        });
        match prefix_of {
            Some(trigger) => {
                format!("no snippet uses this trigger; it is only the start of {trigger}")
            }
            None => "no snippet uses this trigger".to_owned(),
        }
    }
}
