use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use wayexpand_core::{Config, ExpansionEngine, InputEvent, WindowContext};

use crate::editor::Draft;

pub(crate) fn run_command_preview(draft: &Draft) -> Result<String, String> {
    match draft.command_config() {
        Ok(Some(command)) => wayexpand_core::run_command(&command)
            .map_err(|error| format!("Command failed: {error}")),
        Ok(None) => Err("Enable the dynamic command first".into()),
        Err(error) => Err(format!("Command settings invalid: {error}")),
    }
}

pub(crate) fn cache_key(draft: Option<&Draft>, app: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Some(draft) = draft {
        draft.trigger.hash(&mut hasher);
        draft.replacement.hash(&mut hasher);
        draft.match_mode.hash(&mut hasher);
        draft.enabled.hash(&mut hasher);
        draft.propagate_case.hash(&mut hasher);
        draft.app_filter.hash(&mut hasher);
        draft.description.hash(&mut hasher);
        draft.tags.hash(&mut hasher);
        draft.category.hash(&mut hasher);
    }
    app.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn render(
    mut config: Config,
    index: usize,
    draft: Option<&Draft>,
    input: &str,
    app: &str,
) -> String {
    if let Some(draft) = draft {
        config.expansion[index].trigger = draft.trigger.clone();
        config.expansion[index].replacement = draft.replacement.clone();
        config.expansion[index].match_mode = draft.match_mode;
        config.expansion[index].enabled = draft.enabled;
        config.expansion[index].propagate_case = draft.propagate_case;
        config.expansion[index].app_filter = draft
            .app_filter
            .split(',')
            .map(str::trim)
            .filter(|filter| !filter.is_empty())
            .map(str::to_owned)
            .collect();
        config.expansion[index].description = draft.description.clone();
        config.expansion[index].tags = draft
            .tags
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect();
        config.expansion[index].category = draft.category.clone();
        config.expansion[index].command = draft.command_config().ok().flatten();
    }
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
    use wayexpand_core::{ExpansionConfig, MatchMode, OrganizationPolicy, Settings};

    fn config(app_filter: Vec<&str>) -> Config {
        Config {
            expansion: vec![ExpansionConfig {
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
        assert_eq!(render(config(Vec::new()), 0, None, ":hi", ""), "Hello");
    }

    #[test]
    fn app_filter_preview_fails_closed_without_context() {
        assert_eq!(
            render(config(vec!["editor"]), 0, None, ":hi", ""),
            "No expansion matched"
        );
    }

    #[test]
    fn app_filter_preview_uses_selected_application() {
        assert_eq!(
            render(config(vec!["editor"]), 0, None, ":hi", "org.editor"),
            "Hello"
        );
    }

    #[test]
    fn cache_key_changes_when_preview_application_changes() {
        assert_ne!(cache_key(None, "editor"), cache_key(None, "terminal"));
    }

    #[test]
    fn command_preview_requires_explicit_enablement() {
        let expansion = ExpansionConfig {
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
