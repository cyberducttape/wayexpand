use std::{collections::HashMap, sync::Arc};

use wayexpand_core::{
    render_template_with_cursor, Config, ExpansionConfig, ExpansionEngine, InputEvent,
    OrganizationPolicy, Settings, TemplateContext, WindowContext,
};

use crate::editor::Draft;

/// Render a form snippet with editor-supplied values. This local check does
/// not launch the form helper or read environment variables or the clipboard.
pub(crate) fn render_form_template(
    template: &str,
    fields: &HashMap<String, String>,
    snippets: Arc<wayexpand_core::SnippetLibrary>,
) -> Result<(String, Option<usize>), String> {
    let mut context = TemplateContext::system();
    context.fields = Arc::new(fields.clone());
    context.snippets = snippets;
    render_template_with_cursor(template, &context).map_err(|error| error.to_string())
}

/// Return a policy error before a GUI command preview is spawned.
///
/// Without a systemd user manager the GUI runs previews directly, outside any
/// service sandbox, so it must apply the same effective organization
/// enforcement policy first.
pub(crate) fn command_preview_policy_violation(
    policy: &OrganizationPolicy,
    command: &wayexpand_core::CommandConfig,
    sandbox_available: bool,
) -> Option<String> {
    let effective = policy.effective_enforcement_policy();
    if effective.disable_commands {
        return Some("command execution is disabled by organization policy".into());
    }
    // Named actions resolve to administrator-configured executables inside
    // the broker. The GUI has no program path to validate, and must not apply
    // the direct-command absolute-path rule to an action ID.
    if command.action.is_some() {
        return None;
    }
    if let Some(reason) = effective.command_path_violation(&command.program) {
        return Some(reason);
    }
    if effective.safe_mode && !sandbox_available {
        return Some(
            "safe mode requires the systemd service sandbox for direct command previews".into(),
        );
    }
    None
}

#[cfg(test)]
pub(crate) fn run_command_preview(draft: &Draft) -> Result<String, String> {
    match draft.command_config() {
        Ok(Some(command)) => wayexpand_core::run_command(&command)
            .map_err(|error| format!("Command failed: {error}")),
        Ok(None) => Err("Enable the dynamic command first".into()),
        Err(error) => Err(format!("Command settings invalid: {error}")),
    }
}

#[cfg(test)]
pub(crate) fn render(
    source: Option<&ExpansionConfig>,
    settings: &Settings,
    organization: &OrganizationPolicy,
    draft: Option<&Draft>,
    input: &str,
    app: &str,
) -> String {
    render_with_library_snippets(source, settings, organization, draft, input, app, None)
        .unwrap_or_else(|error| format!("Template error: {error}"))
}

pub(crate) fn render_with_library_snippets(
    source: Option<&ExpansionConfig>,
    settings: &Settings,
    organization: &OrganizationPolicy,
    draft: Option<&Draft>,
    input: &str,
    app: &str,
    library_snippets: Option<Arc<wayexpand_core::SnippetLibrary>>,
) -> Result<String, String> {
    let expansion = match draft {
        Some(draft) => match draft.to_expansion(
            source
                .map(|expansion| expansion.id.clone())
                .unwrap_or_else(ExpansionConfig::new_id),
        ) {
            Ok(expansion) => Some(expansion),
            Err(_) => return Err("The current draft has invalid fields".into()),
        },
        None => source.cloned(),
    };
    let Some(expansion) = expansion else {
        return Err("No snippet is selected".into());
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
    let mut engine = ExpansionEngine::new_with_snippets(config, library_snippets)
        .map_err(|error| error.safe_summary())?;
    if !app.trim().is_empty() {
        engine.set_current_window(Some(WindowContext {
            app_id: Some(app.trim().to_owned()),
            title: None,
            instance_id: None,
        }));
    }
    let mut results = engine.process(InputEvent::Text(input.to_owned()));
    results.extend(engine.process(InputEvent::EndOfInput));
    Ok(results
        .last()
        .map(|result| result.insert.clone())
        .unwrap_or_else(|| "No expansion matched".into()))
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
                aliases: Vec::new(),
                output_file: None,
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        }
    }

    #[test]
    fn command_preview_honors_enforced_organization_policy() {
        let mut policy = OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..OrganizationPolicy::default()
        };
        assert_eq!(
            command_preview_policy_violation(&policy, &direct_command("git"), true).as_deref(),
            Some("command execution is disabled by organization policy")
        );

        policy.disable_commands = false;
        policy.require_absolute_commands = true;
        assert!(command_preview_policy_violation(&policy, &direct_command("git"), true).is_some());
        assert!(
            command_preview_policy_violation(&policy, &direct_command("/usr/bin/git"), true)
                .is_none()
        );
    }

    #[test]
    fn safe_mode_blocks_direct_previews_without_the_service_sandbox() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            ..OrganizationPolicy::default()
        };
        assert_eq!(
            command_preview_policy_violation(&policy, &direct_command("/usr/bin/git"), false)
                .as_deref(),
            Some("safe mode requires the systemd service sandbox for direct command previews")
        );
        assert!(
            command_preview_policy_violation(&policy, &direct_command("/usr/bin/git"), true)
                .is_none()
        );
    }

    #[test]
    fn managed_action_uses_broker_path_policy_not_direct_program_path_policy() {
        let policy = OrganizationPolicy {
            safe_mode: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        let mut command = direct_command("");
        command.action = Some("approved-action".into());
        assert!(command_preview_policy_violation(&policy, &command, false).is_none());

        let blocked = OrganizationPolicy {
            safe_mode: true,
            disable_commands: true,
            ..OrganizationPolicy::default()
        };
        assert!(command_preview_policy_violation(&blocked, &command, false).is_some());
    }

    fn direct_command(program: &str) -> wayexpand_core::CommandConfig {
        wayexpand_core::CommandConfig {
            action: None,
            program: program.into(),
            args: Vec::new(),
            timeout_ms: 500,
            cache_ms: 0,
            environment: Default::default(),
            pass_env: Vec::new(),
        }
    }

    #[test]
    fn audit_only_policy_does_not_block_command_preview() {
        let policy = OrganizationPolicy {
            disable_commands: true,
            require_absolute_commands: true,
            ..OrganizationPolicy::default()
        };
        assert!(command_preview_policy_violation(&policy, &direct_command("git"), false).is_none());
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
                Some(&config(vec!["app_id_exact:org.editor"]).expansion[0]),
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
                Some(&config(vec!["app_id_exact:org.editor"]).expansion[0]),
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
            aliases: Vec::new(),
            output_file: None,
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
            "Template error: The current draft has invalid fields"
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
            aliases: Vec::new(),
            output_file: None,
        };
        let draft = Draft::from_expansion(&expansion);
        assert_eq!(
            run_command_preview(&draft),
            Err("Enable the dynamic command first".into())
        );
    }

    fn aliased() -> ExpansionConfig {
        let mut expansion = config(Vec::new()).expansion.remove(0);
        expansion.trigger = ";sig".into();
        expansion.replacement = "Best, Sam".into();
        expansion.aliases = vec![";signature".into(), ";sign".into()];
        expansion
    }

    fn preview(draft: &Draft, input: &str) -> String {
        render(
            Some(&aliased()),
            &Settings::default(),
            &OrganizationPolicy::default(),
            Some(draft),
            input,
            "",
        )
    }

    #[test]
    fn draft_preview_answers_to_the_trigger_and_every_alias() {
        let draft = Draft::from_expansion(&aliased());
        assert_eq!(preview(&draft, ";sig"), "Best, Sam");
        assert_eq!(preview(&draft, ";signature"), "Best, Sam");
        assert_eq!(preview(&draft, ";sign"), "Best, Sam");
    }

    #[test]
    fn draft_preview_includes_a_pending_alias_like_save_does() {
        let mut draft = Draft::from_expansion(&aliased());
        draft.pending_alias = ";bestsam".into();
        assert_eq!(preview(&draft, ";bestsam"), "Best, Sam");
    }

    #[test]
    fn removed_alias_stops_previewing() {
        let mut draft = Draft::from_expansion(&aliased());
        draft.aliases.retain(|alias| alias != ";sign");
        assert_eq!(preview(&draft, ";sign"), "No expansion matched");
        assert_eq!(preview(&draft, ";signature"), "Best, Sam");
    }

    #[test]
    fn invalid_alias_is_reported_invalid() {
        let mut draft = Draft::from_expansion(&aliased());
        draft.pending_alias = ";".repeat(1_000);
        assert!(
            preview(&draft, ";sig").starts_with("Template error: expansion 0 trigger is too long")
        );
    }

    #[test]
    fn draft_preview_includes_a_pending_app_filter_like_save_does() {
        let mut draft = Draft::from_expansion(&aliased());
        draft.pending_app = "app_id_exact:org.editor".into();
        assert_eq!(preview(&draft, ";sig"), "No expansion matched");
    }
}
