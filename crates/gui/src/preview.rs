use wayexpand_core::{
    Config, ExpansionConfig, ExpansionEngine, InputEvent, OrganizationPolicy, Settings,
    WindowContext,
};

use crate::editor::Draft;

#[cfg(test)]
pub(crate) fn run_command_preview(draft: &Draft) -> Result<String, String> {
    match draft.command_config() {
        Ok(Some(command)) => wayexpand_core::run_command(&command)
            .map_err(|error| format!("Command failed: {error}")),
        Ok(None) => Err("Enable the dynamic command first".into()),
        Err(error) => Err(format!("Command settings invalid: {error}")),
    }
}

pub(crate) fn render(
    source: Option<&ExpansionConfig>,
    settings: &Settings,
    organization: &OrganizationPolicy,
    draft: Option<&Draft>,
    input: &str,
    app: &str,
) -> String {
    let expansion = match draft {
        Some(draft) => Some(ExpansionConfig {
            id: source
                .map(|expansion| expansion.id.clone())
                .unwrap_or_else(ExpansionConfig::new_id),
            trigger: draft.trigger.clone(),
            replacement: draft.replacement.clone(),
            description: draft.description.clone(),
            tags: draft.tags.clone(),
            category: draft.category.clone(),
            app_filter: draft.app_filter.clone(),
            match_mode: draft.match_mode,
            command: match draft.command_config() {
                Ok(command) => command,
                Err(_) => return "Configuration is invalid".into(),
            },
            enabled: draft.enabled,
            propagate_case: draft.propagate_case,
        }),
        None => source.cloned(),
    };
    let Some(expansion) = expansion else {
        return "Configuration is invalid".into();
    };
    // A preview is scoped to one expansion. Cloning the production Config and
    // rebuilding its matcher on every preview-input edit made this path O(N)
    // in the entire library despite only one snippet being evaluated.
    let config = Config {
        expansion: vec![expansion],
        hotkey: Vec::new(),
        settings: settings.clone(),
        organization: organization.clone(),
    };
    let Ok(mut engine) = ExpansionEngine::new(config) else {
        return "Configuration is invalid".into();
    };
    if !app.trim().is_empty() {
        engine.set_current_window(Some(WindowContext {
            app_id: Some(app.trim().to_owned()),
            title: None,
        }));
    }
    let mut results = engine.process(InputEvent::Text(input.to_owned()));
    results.extend(engine.process(InputEvent::EndOfInput));
    results
        .last()
        .map(|result| result.insert.clone())
        .unwrap_or_else(|| "No expansion matched".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayexpand_core::{ExpansionConfig, MatchMode};

    fn config(app_filter: Vec<&str>) -> Config {
        Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":hi".into(),
                replacement: "Hello".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: app_filter.into_iter().map(str::to_owned).collect(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        }
    }

    #[test]
    fn renders_committed_trigger_text() {
        let source = config(Vec::new()).expansion.remove(0);
        assert_eq!(
            render(
                Some(&source),
                &Settings::default(),
                &OrganizationPolicy::default(),
                None,
                ":hi",
                ""
            ),
            "Hello"
        );
    }

    #[test]
    fn app_filter_preview_fails_closed_without_context() {
        assert_eq!(
            render(
                Some(&config(vec!["editor"]).expansion[0]),
                &Settings::default(),
                &OrganizationPolicy::default(),
                None,
                ":hi",
                ""
            ),
            "No expansion matched"
        );
    }

    #[test]
    fn app_filter_preview_uses_selected_application() {
        assert_eq!(
            render(
                Some(&config(vec!["editor"]).expansion[0]),
                &Settings::default(),
                &OrganizationPolicy::default(),
                None,
                ":hi",
                "org.editor"
            ),
            "Hello"
        );
    }

    #[test]
    fn invalid_command_draft_does_not_preview_static_replacement() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":hi".into(),
            replacement: "Hello".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        let mut draft = Draft::from_expansion(&expansion);
        draft.command_enabled = true;
        assert_eq!(
            render(
                Some(&config(Vec::new()).expansion[0]),
                &Settings::default(),
                &OrganizationPolicy::default(),
                Some(&draft),
                ":hi",
                ""
            ),
            "Configuration is invalid"
        );
    }

    #[test]
    fn command_preview_requires_explicit_enablement() {
        let expansion = ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":hi".into(),
            replacement: "Hello".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        };
        let draft = Draft::from_expansion(&expansion);
        assert_eq!(
            run_command_preview(&draft),
            Err("Enable the dynamic command first".into())
        );
    }
}
