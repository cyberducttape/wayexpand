use super::*;
use crate::app::insert_at_char_range;
use crate::status::StatusTone;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn run_gui_test_frame(
    ctx: &egui::Context,
    input: egui::RawInput,
    draw: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, draw);
    // Headless tests inspect semantics/layout, not renderer texture uploads.
    // epaint 0.36 asserts if its unapplied font-atlas delta is dropped.
    output.textures_delta.clear();
    output
}

fn import_expansion(trigger: &str, replacement: &str) -> wayexpand_core::ExpansionConfig {
    wayexpand_core::ExpansionConfig {
        id: wayexpand_core::ExpansionConfig::new_id(),
        trigger: trigger.into(),
        replacement: replacement.into(),
        description: String::new(),
        tags: vec!["imported".into()],
        category: String::new(),
        app_filter: Vec::new(),
        match_mode: MatchMode::Immediate,
        command: None,
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    }
}

#[test]
fn route_recommendation_preserves_ibus_topology() {
    let shared_route = RecommendedRoute::IBus;
    let route = route_recommendation(shared_route).expect("the IBus route contract is supported");
    assert_eq!(route.capture, wayexpand_core::BackendKind::InputMethodV2);
    assert_eq!(route.injection, wayexpand_core::BackendKind::InputMethodV2);
    assert_eq!(route.capture_label, "ibus");
    assert_eq!(route.injection_label, "ibus");
    assert!(route.sensitive_fields);
    // Delete and commit are separate IBus signals, so not atomic.
    assert!(!route.atomic_replace);

    let capabilities = Capabilities {
        has_dev_input: true,
        has_direct_libei_socket: true,
        ..Capabilities::default()
    };
    assert!(wayexpand_backend_selection::recommended_route(&capabilities, false, true).is_none());
}

#[test]
fn import_merge_deduplicates_and_keeps_existing_trigger_conflicts() {
    let current = Config {
        expansion: vec![
            import_expansion(":same", "identical"),
            import_expansion(":conflict", "keep this"),
            wayexpand_core::ExpansionConfig {
                trigger: ":hello".into(),
                replacement: "case variant wins".into(),
                propagate_case: true,
                aliases: Vec::new(),
                ..import_expansion(":hello", "case variant wins")
            },
        ],
        hotkey: Vec::new(),
        settings: wayexpand_core::Settings {
            max_buffer_chars: 2048,
            ..Default::default()
        },
        organization: wayexpand_core::OrganizationPolicy::default(),
    };
    let imported = Config {
        expansion: vec![
            import_expansion(":same", "identical"),
            import_expansion(":conflict", "do not replace"),
            import_expansion(":HELLO", "must not collide"),
            import_expansion(":new", "append this"),
        ],
        hotkey: Vec::new(),
        settings: wayexpand_core::Settings::default(),
        organization: wayexpand_core::OrganizationPolicy::default(),
    };

    let (merged, stats) = import::merge_imported_expansions(&current, &imported);
    assert_eq!(stats.added, 1);
    assert_eq!(stats.identical_duplicates, 1);
    assert_eq!(stats.conflicts_kept, 2);
    assert_eq!(merged.expansion.len(), 4);
    assert_eq!(merged.expansion[1].replacement, "keep this");
    assert_eq!(merged.expansion[2].replacement, "case variant wins");
    assert_eq!(merged.settings.max_buffer_chars, 2048);
}

fn draft() -> Draft {
    Draft {
        trigger: ":cmd".into(),
        description: String::new(),
        tags: Vec::new(),
        category: String::new(),
        app_filter: Vec::new(),
        replacement: "fallback".into(),
        enabled: true,
        match_mode: MatchMode::Immediate,
        propagate_case: false,
        command_enabled: true,
        command_action_mode: false,
        command_action: String::new(),
        command_program: "uname".into(),
        command_args: vec!["-s".into(), "-r".into()],
        command_timeout_ms: "500".into(),
        command_cache_ms: "1000".into(),
        command_environment: wayexpand_core::CommandEnvironment::default(),
        command_pass_env: Vec::new(),
        pending_tag: String::new(),
        pending_app: String::new(),
        aliases: Vec::new(),
        pending_alias: String::new(),
    }
}

#[test]
fn a_half_typed_tag_is_saved_and_makes_the_draft_dirty() {
    let mut form = draft();
    form.tags = vec!["ops".into()];
    form.pending_tag = "  release ".into();
    assert_eq!(form.committed_tags(), ["ops", "release"]);
    // An entry that is blank or already present adds nothing.
    form.pending_tag = "ops".into();
    assert_eq!(form.committed_tags(), ["ops"]);
    form.pending_tag = "   ".into();
    assert_eq!(form.committed_tags(), ["ops"]);
    form.pending_app = "firefox".into();
    assert_eq!(form.committed_app_filter(), ["firefox"]);
}

#[test]
fn template_variables_insert_at_the_caret_and_replace_a_selection() {
    let mut text = "Hello world".to_owned();
    assert_eq!(insert_at_char_range(&mut text, Some(5..5), ","), 6);
    assert_eq!(text, "Hello, world");
    assert_eq!(
        insert_at_char_range(&mut text, Some(7..12), "{{cursor}}"),
        17
    );
    assert_eq!(text, "Hello, {{cursor}}");
    // No remembered caret appends; an out-of-range caret is clamped.
    assert_eq!(insert_at_char_range(&mut text, None, "!"), 18);
    assert_eq!(insert_at_char_range(&mut text, Some(99..120), "?"), 19);
    assert_eq!(text, "Hello, {{cursor}}!?");
    // Char indices, not bytes: multi-byte text before the caret.
    let mut text = "ä€x".to_owned();
    assert_eq!(insert_at_char_range(&mut text, Some(2..2), "-"), 3);
    assert_eq!(text, "ä€-x");
}

#[test]
fn status_bar_paths_are_shown_relative_to_home() {
    let home = std::ffi::OsStr::new("/home/ada");
    assert_eq!(
        home_relative_path(Path::new("/home/ada/.config/wayexpand/x.toml"), Some(home)),
        "~/.config/wayexpand/x.toml"
    );
    assert_eq!(
        home_relative_path(Path::new("/etc/wayexpand/x.toml"), Some(home)),
        "/etc/wayexpand/x.toml"
    );
    assert_eq!(
        home_relative_path(Path::new("/home/adam/x.toml"), Some(home)),
        "/home/adam/x.toml"
    );
}

#[test]
fn command_editor_builds_direct_program_configuration() {
    let command = draft().command_config().unwrap().unwrap();
    assert_eq!(command.program, "uname");
    assert_eq!(command.args, ["-s", "-r"]);
    assert_eq!(command.timeout_ms, 500);
    assert_eq!(command.cache_ms, 1000);
}

#[test]
fn command_editor_round_trips_managed_action_configuration() {
    let source = wayexpand_core::ExpansionConfig {
        id: wayexpand_core::ExpansionConfig::new_id(),
        trigger: ":cluster".into(),
        replacement: String::new(),
        description: String::new(),
        tags: Vec::new(),
        category: String::new(),
        app_filter: Vec::new(),
        match_mode: MatchMode::Immediate,
        command: Some(wayexpand_core::CommandConfig {
            action: Some("cluster-status".into()),
            program: String::new(),
            args: Vec::new(),
            timeout_ms: 3000,
            cache_ms: 0,
            environment: wayexpand_core::CommandEnvironment::Minimal,
            pass_env: vec!["KUBECONFIG".into()],
        }),
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    };
    let form = Draft::from_expansion(&source);
    assert!(form.command_action_mode);
    assert!(form.matches_command(source.command.as_ref()));
    assert_eq!(form.command_config().unwrap(), source.command);
}

#[test]
fn command_editor_preserves_advanced_environment_settings() {
    let mut source = wayexpand_core::ExpansionConfig {
        id: wayexpand_core::ExpansionConfig::new_id(),
        trigger: ":foo".into(),
        replacement: String::new(),
        description: String::new(),
        tags: Vec::new(),
        category: String::new(),
        app_filter: Vec::new(),
        match_mode: MatchMode::Immediate,
        command: None,
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    };
    source.command = Some(wayexpand_core::CommandConfig {
        action: None,
        program: "/usr/bin/foo ".into(),
        args: vec![String::new(), " foo ".into(), "hello\nworld".into()],
        timeout_ms: 900,
        cache_ms: 0,
        environment: wayexpand_core::CommandEnvironment::Inherit,
        pass_env: vec![" DISPLAY ".into(), "TEAM\nID".into()],
    });
    let mut form = Draft::from_expansion(&source);
    assert!(form.matches_command(source.command.as_ref()));
    assert_eq!(form.command_args, source.command.as_ref().unwrap().args);
    assert_eq!(
        form.command_pass_env,
        source.command.as_ref().unwrap().pass_env
    );
    form.description = "Edited description".into();
    assert!(form.matches_command(source.command.as_ref()));
    assert_eq!(form.command_config().unwrap(), source.command);
    form.command_program.clear();
    assert!(!form.matches_command(source.command.as_ref()));
    assert!(form.command_config().is_err());
}

#[test]
fn new_snippet_stays_out_of_config_until_a_nonempty_replacement_is_saved() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-new-draft-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    let original_file = fs::read(&path).unwrap();

    app.create_new_snippet();

    assert!(app.new_draft);
    assert!(app.draft_is_dirty());
    assert!(app.config.expansion.is_empty());
    assert_eq!(fs::read(&path).unwrap(), original_file);
    assert!(app.draft.as_ref().unwrap().replacement.is_empty());
    assert!(app.draft.as_ref().unwrap().enabled);

    app.save_selected();
    assert!(app.new_draft);
    assert!(app.config.expansion.is_empty());
    assert_eq!(fs::read(&path).unwrap(), original_file);

    app.draft.as_mut().unwrap().replacement = "Finished snippet".into();
    assert_eq!(app.preview(), "Finished snippet");
    app.save_selected();

    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.expansion.len(), 1);
    assert_eq!(saved.expansion[0].trigger, ":new");
    assert_eq!(saved.expansion[0].replacement, "Finished snippet");
    assert!(saved.expansion[0].enabled);
    assert!(!app.new_draft);
    assert!(!app.draft_is_dirty());
    fs::remove_file(path).unwrap();
}

#[test]
fn the_save_shortcut_saves_a_new_snippet_and_skips_a_clean_draft() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-save-shortcut-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();

    app.create_new_snippet();
    app.draft.as_mut().unwrap().replacement = "Typed with Ctrl+S".into();
    app.save_shortcut();
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.expansion.len(), 1);
    assert_eq!(saved.expansion[0].replacement, "Typed with Ctrl+S");

    let undo_depth = app.undo.len();
    let file = fs::read(&path).unwrap();
    app.save_shortcut();
    assert_eq!(fs::read(&path).unwrap(), file);
    assert_eq!(app.undo.len(), undo_depth);
    fs::remove_file(path).unwrap();
}

#[test]
fn discarding_a_new_snippet_restores_the_previous_selection_without_writing() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-discard-new-draft-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![import_expansion(":existing", "Existing text")],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    let original_file = fs::read(&path).unwrap();

    app.create_new_snippet();
    app.draft.as_mut().unwrap().replacement = "discard me".into();
    app.request_action(PendingAction::Select(0));
    assert_eq!(app.pending_action, Some(PendingAction::Select(0)));
    app.discard_pending();

    assert!(!app.new_draft);
    assert_eq!(app.selected_index(), Some(0));
    assert_eq!(app.draft.as_ref().unwrap().trigger, ":existing");
    assert_eq!(app.config.expansion.len(), 1);
    assert_eq!(fs::read(&path).unwrap(), original_file);
    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_command_draft_is_not_mistaken_for_clean_none() {
    let source = wayexpand_core::ExpansionConfig {
        id: wayexpand_core::ExpansionConfig::new_id(),
        trigger: ":foo".into(),
        replacement: String::new(),
        description: String::new(),
        tags: Vec::new(),
        category: String::new(),
        app_filter: Vec::new(),
        match_mode: MatchMode::Immediate,
        command: None,
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    };
    let mut form = Draft::from_expansion(&source);
    form.command_enabled = true;
    assert!(!form.matches_command(None));
    assert!(form.command_config().is_err());
}

#[test]
fn editor_draft_preserves_tag_and_app_filter_tokens_verbatim() {
    let source = wayexpand_core::ExpansionConfig {
        id: wayexpand_core::ExpansionConfig::new_id(),
        trigger: ":foo".into(),
        replacement: String::new(),
        description: String::new(),
        tags: vec!["customer, west".into(), " email ".into()],
        category: String::new(),
        app_filter: vec!["org.example, beta".into(), "browser".into()],
        match_mode: MatchMode::Immediate,
        command: None,
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    };
    let form = Draft::from_expansion(&source);
    assert_eq!(form.tags, source.tags);
    assert_eq!(form.app_filter, source.app_filter);
}

#[test]
fn command_editor_rejects_non_numeric_limits() {
    let mut draft = draft();
    draft.command_timeout_ms = "half a second".into();
    let error = draft.command_config().unwrap_err().to_string();
    assert!(error.contains("timeout must be an integer"));
}

#[test]
fn disabled_command_editor_removes_command() {
    let mut draft = draft();
    draft.command_enabled = false;
    assert!(draft.command_config().unwrap().is_none());
}

#[test]
fn cancelling_app_detection_keeps_the_worker_until_it_finishes() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-detection-{}.toml",
        std::process::id()
    ));
    let mut app = GuiApp::load(path.clone()).unwrap();
    let (_sender, receiver) = mpsc::channel();
    app.app_detection = Some(AppDetectionTask {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
    });

    app.cancel_app_detection();

    assert!(app
        .app_detection
        .as_ref()
        .is_some_and(|task| task.cancelled.load(Ordering::Acquire)));
    let _ = fs::remove_file(path);
}

#[test]
fn missing_configuration_is_initialized_without_replacing_existing_files() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-onboarding-run-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let app = GuiApp::load(path.clone()).unwrap();
    assert!(app.config.expansion.is_empty());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn switching_snippets_preserves_unsaved_draft_until_decision() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-dirty-{}.toml", std::process::id()));
    let config = Config {
        expansion: vec![
            ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":one".into(),
                replacement: "one".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
            },
            ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":two".into(),
                replacement: "two".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
            },
        ],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.draft.as_mut().unwrap().replacement = "changed".into();
    app.request_action(PendingAction::Select(1));
    assert_eq!(app.selected, Some(0));
    assert!(matches!(app.pending_action, Some(PendingAction::Select(1))));
    app.discard_pending();
    assert_eq!(app.selected, Some(1));
    assert!(app.pending_action.is_none());
    app.duplicate_selected();
    assert_eq!(app.config.expansion.len(), 3);
    assert_eq!(app.config.expansion[2].trigger, ":two-copy");
    fs::remove_file(path).unwrap();
}

#[test]
fn gui_refuses_to_overwrite_an_external_config_change() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-external-change-{}.toml",
        std::process::id()
    ));
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":one".into(),
            replacement: "one".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.draft.as_mut().unwrap().replacement = "from gui".into();

    let mut external = config.clone();
    external.expansion[0].replacement = "from external editor".into();
    external.save_atomic(&path).unwrap();

    app.save_selected();

    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "from external editor"
    );
    assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
    assert!(app.status.text().contains("changed outside WayExpand"));
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_reload_leaves_the_loaded_editor_snapshot_untouched() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-failed-reload-{}.toml",
        std::process::id()
    ));
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":before".into(),
            replacement: "original text".into(),
            description: "original description".into(),
            tags: vec!["preserve".into()],
            category: "original category".into(),
            app_filter: vec!["org.example.Editor".into()],
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    let original_config = app.config.clone();
    let original_document = app.config_document.to_string();
    let original_revision = app.config_revision.clone();
    let original_selected_id = app.selected_id.clone();
    let original_draft = app.draft.as_ref().unwrap().clone();
    fs::write(&path, "[[expansion]\nthis is not valid TOML").unwrap();

    let (sender, diagnostics_sender, receiver) = runtime::start().unwrap();
    app.runtime_sender = Some(sender);
    app.diagnostics_sender = Some(diagnostics_sender);
    app.runtime_receiver = Some(receiver);
    app.perform_reload();
    assert!(app.pending_reload_revision.is_some());
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.pending_reload_revision.is_some() {
        app.poll_runtime(&ctx);
        assert!(
            Instant::now() < deadline,
            "background reload did not finish"
        );
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        format!("{:?}", app.config),
        format!("{:?}", original_config)
    );
    assert_eq!(app.config_document.to_string(), original_document);
    assert_eq!(app.config_revision, original_revision);
    assert_eq!(app.selected_id, original_selected_id);
    let draft = app.draft.as_ref().unwrap();
    assert_eq!(draft.trigger, original_draft.trigger);
    assert_eq!(draft.description, original_draft.description);
    assert_eq!(draft.replacement, original_draft.replacement);
    assert_eq!(draft.tags, original_draft.tags);
    assert_eq!(draft.app_filter, original_draft.app_filter);
    assert_eq!(app.preview_input, original_draft.trigger);
    assert_eq!(app.selected, Some(0));
    assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);

    fs::remove_file(path).unwrap();
}

#[test]
fn reload_completion_cannot_overwrite_edits_made_while_loading() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-reload-race-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":before".into(),
            replacement: "original".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    let original_revision = app.config_revision.clone();

    let mut external_config = config;
    external_config.expansion[0].replacement = "external version".into();
    external_config.save_atomic(&path).unwrap();
    let loaded = Config::load_versioned(&path).unwrap();
    let snapshot = runtime::ReloadSnapshot {
        document: persistence::read_config_document(loaded.source()).unwrap(),
        search_index: library::SearchIndex::new(&loaded.config),
        revision: loaded.revision,
        config: loaded.config,
    };
    app.draft.as_mut().unwrap().replacement = "local unsaved edit".into();

    let (completion_sender, completion_receiver) = mpsc::channel();
    app.runtime_receiver = Some(completion_receiver);
    app.pending_reload_revision = Some(original_revision.clone());
    app.pending_control = 1;
    completion_sender
        .send(runtime::Completion::ConfigReloaded(Box::new(Ok(snapshot))))
        .unwrap();
    app.poll_runtime(&egui::Context::default());

    assert_eq!(app.config_revision, original_revision);
    assert_eq!(app.config.expansion[0].replacement, "original");
    assert_eq!(
        app.draft.as_ref().unwrap().replacement,
        "local unsaved edit"
    );
    assert_eq!(app.status.tone_for_test(), StatusTone::Warning);
    fs::remove_file(path).unwrap();
}

#[test]
fn gui_core_revision_guard_catches_a_write_after_the_early_check() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-revision-race-{}.toml",
        std::process::id()
    ));
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":one".into(),
            replacement: "initial".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let app = GuiApp::load(path.clone()).unwrap();
    let mut external = config.clone();
    external.expansion[0].replacement = "external edit".into();
    external.save_atomic(&path).unwrap();

    let mut stale_candidate = app.config.clone();
    stale_candidate.expansion[0].replacement = "stale GUI edit".into();
    let replacement = toml_edit::ser::to_document(&stale_candidate).unwrap();
    let document = persistence::merge_config_document(app.config_document.clone(), replacement);
    assert!(Config::save_atomic_text_if_revision_matches(
        &path,
        &document.to_string(),
        &app.config_revision,
    )
    .unwrap_err()
    .safe_summary()
    .contains("changed externally"));
    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "external edit"
    );
    assert_eq!(app.config.expansion[0].replacement, "initial");
    fs::remove_file(path).unwrap();
}

#[test]
fn import_path_expands_home_prefix_without_shell_evaluation() {
    assert_eq!(
        expand_user_path_with_home(
            "~/matches.yml",
            Some(std::ffi::OsStr::new("/tmp/wayexpand-home")),
        ),
        PathBuf::from("/tmp/wayexpand-home/matches.yml")
    );
    assert_eq!(
        expand_user_path_with_home(
            "/tmp/matches.yml",
            Some(std::ffi::OsStr::new("/tmp/wayexpand-home")),
        ),
        PathBuf::from("/tmp/matches.yml")
    );
    assert_eq!(
        expand_user_path_with_home("~/matches.yml", None),
        PathBuf::from("~/matches.yml")
    );
}

#[test]
fn undo_history_is_bounded() {
    let path = std::env::temp_dir().join(format!("wayexpand-gui-undo-{}.toml", std::process::id()));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    for _ in 0..(MAX_UNDO_HISTORY + 8) {
        app.remember_undo(Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":undo-test".into(),
                replacement: "previous value".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        });
    }
    assert_eq!(app.undo.len(), MAX_UNDO_HISTORY);
    assert!(app.undo_bytes <= MAX_UNDO_BYTES);
    assert_eq!(
        app.undo_bytes,
        app.undo
            .iter()
            .map(UndoEntry::estimated_bytes)
            .sum::<usize>()
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn undo_history_respects_its_byte_budget() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-undo-bytes-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    for _ in 0..=MAX_UNDO_HISTORY {
        app.remember_undo(Config {
            expansion: vec![ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":large-undo".into(),
                replacement: "x".repeat(300_000),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
            }],
            hotkey: Vec::new(),
            settings: Settings::default(),
            organization: OrganizationPolicy::default(),
        });
    }
    assert!(app.undo.len() < MAX_UNDO_HISTORY);
    assert!(app.undo_bytes <= MAX_UNDO_BYTES);
    fs::remove_file(path).unwrap();
}

#[test]
fn saved_snippet_edit_can_be_undone_back_to_disk() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-undo-save-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":undo-save".into(),
            replacement: "before".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.config.expansion[0].replacement = "after".into();
    let previous = Config::load(&path).unwrap();
    app.remember_undo(previous);

    app.undo();

    assert_eq!(app.config.expansion[0].replacement, "before");
    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "before"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn undo_is_refused_while_a_save_is_in_flight() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-undo-in-flight-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":undo-flight".into(),
            replacement: "before".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.config.expansion[0].replacement = "after".into();
    let previous = Config::load(&path).unwrap();
    app.remember_undo(previous);

    // A second undo click while the first undo's save is still running must
    // not queue another undo computed from the same entry; finishing that
    // stale request used to pop an empty history and panic.
    let mut candidate = app.config.clone();
    candidate.expansion[0].replacement = "before".into();
    app.pending_save = Some(PendingSave {
        request_id: 7,
        candidate: candidate.clone(),
        preview_revision: app.preview_revision,
        intent: SaveIntent::Undo,
    });
    app.undo();
    assert!(app.queued_save.is_none());
    assert_eq!(app.undo.len(), 1);

    let completed = (app.config_revision.clone(), app.config_document.clone());
    app.finish_save(7, Ok(completed));
    assert!(app.undo.is_empty());
    assert_eq!(app.config.expansion[0].replacement, "before");
    fs::remove_file(path).unwrap();
}

#[test]
fn a_save_that_finishes_after_newer_edits_still_updates_the_loaded_config() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-save-race-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":race".into(),
            replacement: "before".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();

    // Simulate the background save landing after the user kept typing.
    let mut candidate = app.config.clone();
    candidate.expansion[0].replacement = "saved".into();
    app.pending_save = Some(PendingSave {
        request_id: 41,
        candidate: candidate.clone(),
        preview_revision: app.preview_revision,
        intent: SaveIntent::Snippet {
            is_new: false,
            index: 0,
        },
    });

    // Two edits arriving while the first write is in flight coalesce to
    // the newest desired state instead of being rejected as "busy".
    let mut queued = app.config.clone();
    queued.expansion[0].replacement = "newest".into();
    assert!(app.queue_save_config(
        queued,
        SaveIntent::Snippet {
            is_new: false,
            index: 0,
        }
    ));
    let mut latest = app.config.clone();
    latest.expansion[0].replacement = "latest".into();
    assert!(app.queue_save_config(
        latest,
        SaveIntent::Snippet {
            is_new: false,
            index: 0,
        }
    ));
    assert_eq!(
        app.queued_save.as_ref().unwrap().candidate.expansion[0].replacement,
        "latest"
    );
    app.invalidate_preview();
    let result = runtime::save_config(
        path.clone(),
        candidate,
        app.config_document.clone(),
        app.config_revision.clone(),
    );
    app.finish_save(41, result);

    assert_eq!(app.config.expansion[0].replacement, "latest");
    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "latest"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn undo_delta_restores_modified_deleted_inserted_and_reordered_snippets() {
    let existing = ExpansionConfig {
        id: "stable-a".into(),
        trigger: ":before".into(),
        replacement: "old text".into(),
        description: String::new(),
        tags: Vec::new(),
        category: String::new(),
        app_filter: Vec::new(),
        match_mode: MatchMode::Immediate,
        command: None,
        enabled: true,
        propagate_case: false,
        aliases: Vec::new(),
    };
    let deleted = ExpansionConfig {
        id: "stable-b".into(),
        trigger: ":deleted".into(),
        replacement: "restore me".into(),
        aliases: Vec::new(),
        ..existing.clone()
    };
    let mut modified = existing.clone();
    modified.trigger = ":after".into();
    modified.replacement = "new text".into();
    let inserted = ExpansionConfig {
        id: "stable-c".into(),
        trigger: ":inserted".into(),
        aliases: Vec::new(),
        ..existing.clone()
    };
    let previous = Config {
        expansion: vec![existing.clone(), deleted.clone()],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let current = Config {
        expansion: vec![inserted, modified],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };

    let entry = UndoEntry::between(&previous, &current).unwrap();
    let restored = entry.restore(&current).unwrap();
    assert_eq!(restored.expansion, [existing, deleted]);
}

#[test]
fn settings_editor_rejects_out_of_range_buffer_limit() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-settings-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.settings_buffer = "0".into();
    app.save_settings(&egui::Context::default());
    assert_eq!(app.config.settings.max_buffer_chars, 128);
    assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);
    assert!(app.status.text().contains("outside the allowed range"));
    // The dialog keeps the reason next to the field that caused it, not
    // only on the status line at the far edge of the window.
    assert!(app
        .settings_error
        .as_deref()
        .is_some_and(|error| error.contains("outside the allowed range")));
    fs::remove_file(path).unwrap();
}

#[test]
fn a_non_numeric_buffer_limit_is_reported_without_touching_the_configuration() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-buffer-{}.toml", std::process::id()));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    let before = app.config.settings.max_buffer_chars;
    app.settings_open = true;

    app.settings_buffer = "half a screen".into();
    app.save_settings(&egui::Context::default());

    assert_eq!(app.config.settings.max_buffer_chars, before);
    assert_eq!(app.status.tone_for_test(), status::StatusTone::Error);
    // The dialog stays open so the value can be corrected in place.
    assert!(app.settings_open);
    fs::remove_file(path).unwrap();
}

#[test]
fn escape_closes_one_dialog_at_a_time_and_keeps_a_loaded_import() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-dialogs-{}.toml", std::process::id()));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.diagnostics_open = true;
    app.import_open = true;
    app.import_preview = Some((
        app.config.clone(),
        wayexpand_core::EspansoImportReport::default(),
    ));
    app.settings_open = true;
    app.pending_action = Some(PendingAction::Close);
    assert!(app.any_dialog_open());

    // The modal save/discard prompt is answered first, as Cancel, and
    // leaves the windows beneath it open.
    app.close_topmost_dialog();
    assert!(app.pending_action.is_none());
    assert!(app.settings_open);

    app.close_topmost_dialog();
    assert!(!app.settings_open);
    assert!(app.import_open);

    // The first Escape on the import dialog drops the preview, not the
    // dialog: a loaded library is expensive to reproduce.
    app.close_topmost_dialog();
    assert!(app.import_open);
    assert!(app.import_preview.is_none());

    app.close_topmost_dialog();
    assert!(!app.import_open);
    assert!(app.diagnostics_open);

    app.close_topmost_dialog();
    assert!(!app.any_dialog_open());
    fs::remove_file(path).unwrap();
}

#[test]
fn diagnostics_dialog_stays_inside_a_small_window() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-diagnostics-fit-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.diagnostics_open = true;
    app.daemon_reachable = Some(true);
    app.daemon_capabilities = Some(runtime::DaemonCapabilities::default());
    app.daemon_status = "x".repeat(400);
    app.protocol_probes = (0..40)
        .map(|index| (format!("probe-{index}"), "detail ".repeat(30)))
        .collect();

    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
    let ctx = egui::Context::default();
    // Windows settle their size over a few frames.
    for _ in 0..4 {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = run_gui_test_frame(&ctx, input, |ui| {
            let frame_ctx = ui.ctx().clone();
            let palette = Palette::for_pack(app.colorpack, app.dark_mode);
            app.render_diagnostics(&frame_ctx, &palette);
        });
    }
    let window = ctx
        .memory(|memory| memory.area_rect(egui::Id::new(Some(app.strings.diagnostics_title()))))
        .expect("diagnostics window was laid out");
    assert!(
        screen.expand(1.0).contains_rect(window),
        "diagnostics window {window:?} overflows the {screen:?} viewport"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn the_window_title_names_the_open_file_and_marks_unsaved_edits() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-title-{}.toml", std::process::id()));
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":one".into(),
            replacement: "one".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    let ctx = egui::Context::default();

    app.sync_window_title(&ctx);
    let clean = app.window_title.clone();
    assert!(clean.contains(path.file_name().unwrap().to_str().unwrap()));
    assert!(!clean.starts_with('•'));

    app.draft.as_mut().unwrap().replacement = "changed".into();
    app.sync_window_title(&ctx);
    assert!(app.window_title.starts_with('•'));

    fs::remove_file(path).unwrap();
}

#[test]
fn a_font_scale_change_persists_without_consuming_the_undo_history() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-fontscale-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    let ctx = egui::Context::default();

    app.apply_font_scale(&ctx, FontScale::Large);

    assert_eq!(app.config.settings.font_scale, FontScale::Normal);
    assert_eq!(app.settings_font_scale, FontScale::Large);
    assert!(app.undo.is_empty());
    fs::remove_file(path).unwrap();
}

/// Renders every panel and dialog headlessly. Layout code is not
/// otherwise exercised by the unit tests, so this is what catches an
/// out-of-range index, a mismatched `Grid`/`ScrollArea` id, or a
/// borrow-order mistake in a dialog that is only reachable by clicking.
#[test]
fn every_panel_and_dialog_renders_for_both_languages_and_settings_tabs() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-render-{}.toml", std::process::id()));
    let config = Config {
        expansion: vec![
            ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":plain".into(),
                replacement: "plain text".into(),
                description: "A plain snippet".into(),
                tags: vec!["demo".into()],
                category: "email".into(),
                app_filter: vec!["konsole".into()],
                match_mode: MatchMode::Immediate,
                command: None,
                enabled: true,
                propagate_case: false,
                aliases: Vec::new(),
            },
            ExpansionConfig {
                id: ExpansionConfig::new_id(),
                trigger: ":cmd".into(),
                replacement: "fallback".into(),
                description: String::new(),
                tags: Vec::new(),
                category: String::new(),
                app_filter: Vec::new(),
                match_mode: MatchMode::WordBoundary,
                command: Some(wayexpand_core::CommandConfig {
                    action: None,
                    program: "uname".into(),
                    args: vec!["-s".into()],
                    timeout_ms: 500,
                    cache_ms: 0,
                    environment: wayexpand_core::CommandEnvironment::default(),
                    pass_env: Vec::new(),
                }),
                enabled: false,
                propagate_case: true,
                aliases: Vec::new(),
            },
        ],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let _ = fs::remove_file(&path);
    config.save_atomic(&path).unwrap();
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.diagnostics_open = true;
    app.daemon_reachable = Some(true);
    app.daemon_capabilities = Some(runtime::DaemonCapabilities {
        injection_mode: Some("ei_text"),
        injection_max_text_chars: None,
        injection_throughput_chars_per_sec: None,
        capture_sensitive_focus: Some(false),
        capture_exclusive: Some(false),
        capture_reliable_key_state: Some(true),
        capture_key_passthrough: Some(false),
        capture_composition_aware: Some(false),
        capture_local_compose_aware: Some(false),
        capture_layout_aware: Some(false),
        window_tracker_connected: Some(true),
        window_identity_exact: Some(true),
        inject_atomic_replace: Some(false),
        inject_full_unicode: Some(true),
        inject_cursor_reposition: Some(true),
        inject_key_passthrough: Some(true),
    });
    app.import_open = true;
    app.settings_open = true;
    app.pending_action = Some(PendingAction::Delete);

    let ctx = egui::Context::default();
    for language in [Language::English, Language::German] {
        // Set the pair directly rather than through `set_language`: that
        // would persist to the real user preferences file.
        app.language = language;
        app.strings.set_language(language);
        for tab in [SettingsTab::Appearance, SettingsTab::Engine] {
            app.settings_tab = tab;
            for width in [420.0, 640.0, 980.0] {
                for selected in [Some(0), Some(1), None] {
                    app.set_selected_index(selected);
                    app.draft =
                        selected.map(|index| Draft::from_expansion(&app.config.expansion[index]));
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        ..Default::default()
                    };
                    let _ = run_gui_test_frame(&ctx, input, |ui| {
                        let frame_ctx = ui.ctx().clone();
                        let palette = Palette::for_pack(app.colorpack, app.dark_mode);
                        app.sync_window_title(&frame_ctx);
                        app.render_toolbar(ui, &palette);
                        app.render_diagnostics(&frame_ctx, &palette);
                        app.render_import_dialog(&frame_ctx, &palette);
                        app.render_settings_dialog(&frame_ctx, &palette);
                        app.render_status_bar(ui, &palette);
                        app.render_snippet_list(ui, &palette);
                        app.render_editor_actions(ui, &palette);
                        app.render_editor(ui, &palette);
                        app.render_pending_action(&frame_ctx, &palette);
                    });
                }
            }
        }
    }
    // Exercise the narrowest supported desktop window with 200% text;
    // this is a render smoke test, not a pixel-perfect clipping oracle.
    theme::install_pack(&ctx, app.colorpack, FontScale::Huge);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(320.0, 480.0),
        )),
        ..Default::default()
    };
    let _ = run_gui_test_frame(&ctx, input, |ui| {
        let frame_ctx = ui.ctx().clone();
        let palette = Palette::for_pack(app.colorpack, app.dark_mode);
        app.render_toolbar(ui, &palette);
        app.render_diagnostics(&frame_ctx, &palette);
        app.render_import_dialog(&frame_ctx, &palette);
        app.render_settings_dialog(&frame_ctx, &palette);
        app.render_status_bar(ui, &palette);
        app.render_snippet_list(ui, &palette);
        app.render_editor_actions(ui, &palette);
        app.render_editor(ui, &palette);
        app.render_pending_action(&frame_ctx, &palette);
    });
    app.create_new_snippet();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(420.0, 760.0),
        )),
        ..Default::default()
    };
    let _ = run_gui_test_frame(&ctx, input, |ui| {
        let palette = Palette::for_pack(app.colorpack, app.dark_mode);
        app.render_editor_actions(ui, &palette);
        app.render_editor(ui, &palette);
    });
    assert!(app.new_draft);
    assert_eq!(app.config.expansion.len(), 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn first_run_screen_renders_and_creates_a_real_test_expansion() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-first-run-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    assert!(app.config.expansion.is_empty());
    let ctx = egui::Context::default();
    for language in [Language::English, Language::German] {
        app.language = language;
        app.strings.set_language(language);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = run_gui_test_frame(&ctx, input, |ui| {
            let palette = Palette::for_pack(app.colorpack, app.dark_mode);
            app.render_editor(ui, &palette);
        });
    }
    app.create_test_snippet();
    assert_eq!(app.config.expansion.len(), 1, "{}", app.status.text());
    assert_eq!(app.config.expansion[0].trigger, ":wayexpand-test");
    assert_eq!(app.config.expansion[0].replacement, "WayExpand is working!");
    app.filter = "working".into();
    app.refresh_visible_indices_cache();
    assert!(app.visible_indices().is_empty());
    app.search_fields.replacements = true;
    app.refresh_visible_indices_cache();
    assert_eq!(app.visible_indices(), vec![0]);
    assert_eq!(Config::load(&path).unwrap().expansion.len(), 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn raw_input_setup_command_is_hidden_until_explicit_acknowledgement() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-evdev-consent-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.evdev_setup_open = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let render = |app: &mut GuiApp| {
        let output = run_gui_test_frame(&ctx, egui::RawInput::default(), |ui| {
            app.render_evdev_setup(
                &ui.ctx().clone(),
                &Palette::for_pack(app.colorpack, app.dark_mode),
            );
        });
        format!(
            "{:?}",
            output.platform_output.accesskit_update.unwrap().nodes
        )
    };

    let before_acknowledgement = render(&mut app);
    assert!(before_acknowledgement.contains("password-field"));
    assert!(!before_acknowledgement.contains("wayexpand setup --mode maximum"));

    app.evdev_setup_acknowledged = true;
    let after_acknowledgement = render(&mut app);
    assert!(after_acknowledgement.contains("wayexpand setup --mode maximum"));
    assert!(after_acknowledgement.contains("not run by the GUI"));
    fs::remove_file(path).unwrap();
}

#[test]
fn a_selection_left_behind_by_a_shrinking_library_is_reported_not_indexed() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-gui-stale-{}.toml", std::process::id()));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    // A configuration reloaded from disk (or replaced by an import) can
    // be shorter than the one the selection was made against.
    app.selected = Some(4);

    assert!(app.selected_index().is_none());
    assert!(!app.draft_is_dirty());
    app.save_selected();
    app.duplicate_selected();
    app.perform_delete_selected();
    app.toggle_enabled(4);

    assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
    assert!(app.config.expansion.is_empty());
    fs::remove_file(path).unwrap();
}

#[test]
fn switching_language_retranslates_the_status_line_wording() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-language-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();

    app.language = Language::German;
    app.strings.set_language(Language::German);
    app.undo.clear();
    app.undo();

    assert_eq!(app.status.tone_for_test(), status::StatusTone::Warning);
    assert_eq!(app.status.text(), "Nichts zum Rückgängigmachen");
    fs::remove_file(path).unwrap();
}

#[test]
fn format_preserving_saves_keep_manual_comments() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-format-preservation-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: vec![ExpansionConfig {
            id: ExpansionConfig::new_id(),
            trigger: ":sig".into(),
            replacement: "Regards".into(),
            description: "Signature".into(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
            aliases: Vec::new(),
        }],
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    let text = format!(
        "# Maintained by the team; keep this note.\n\n{}",
        toml_edit::ser::to_string_pretty(&config).unwrap()
    );
    fs::write(&path, text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let app = GuiApp::load(path.clone()).unwrap();
    let mut candidate = app.config.clone();
    candidate.expansion[0].replacement = "Best regards".into();

    let replacement = toml_edit::ser::to_document(&candidate).unwrap();
    let document = persistence::merge_config_document(app.config_document.clone(), replacement);
    Config::save_atomic_text_if_revision_matches(
        &path,
        &document.to_string(),
        &app.config_revision,
    )
    .unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.starts_with("# Maintained by the team; keep this note."));
    assert!(saved.contains("replacement = \"Best regards\""));
    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "Best regards"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn format_merge_matches_duplicate_triggers_in_order() {
    let old = persistence::read_config_document(
        "[[expansion]]\n# disabled duplicate one\ntrigger = ':dup'\nenabled = false\n\n[[expansion]]\n# disabled duplicate two\ntrigger = ':dup'\nenabled = false\n",
    )
    .unwrap();
    let new = persistence::read_config_document(
        "[[expansion]]\ntrigger = ':dup'\nenabled = false\n\n[[expansion]]\ntrigger = ':dup'\nenabled = false\n",
    )
    .unwrap();
    let merged = persistence::merge_config_document(old, new).to_string();
    assert!(
        merged.find("# disabled duplicate one").unwrap()
            < merged.find("# disabled duplicate two").unwrap()
    );
    assert_eq!(merged.matches("trigger = ':dup'").count(), 2);
}

#[test]
fn format_merge_follows_stable_ids_when_triggers_change_and_reorder() {
    let old = persistence::read_config_document(
        "# note for A\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000001'\ntrigger = ':old-a'\n\n# note for B\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000002'\ntrigger = ':old-b'\n",
    )
    .unwrap();
    let new = persistence::read_config_document(
        "[[expansion]]\nid = '00000000-0000-4000-8000-000000000002'\ntrigger = ':new-b'\n\n[[expansion]]\nid = '00000000-0000-4000-8000-000000000001'\ntrigger = ':new-a'\n",
    )
    .unwrap();

    let merged = persistence::merge_config_document(old, new).to_string();
    let b_id = merged.find("00000000-0000-4000-8000-000000000002").unwrap();
    let b_note = merged.find("# note for B").unwrap();
    let a_id = merged.find("00000000-0000-4000-8000-000000000001").unwrap();
    let a_note = merged.find("# note for A").unwrap();
    assert!(b_note < b_id && b_id < a_note && a_note < a_id, "{merged}");
    assert!(merged.contains("trigger = ':new-b'"));
    assert!(merged.contains("trigger = ':new-a'"));
}

#[test]
fn gui_selection_tracks_the_snippet_id_when_config_order_changes() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-gui-selection-id-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let mut app = GuiApp::load(path.clone()).unwrap();
    app.config.expansion = vec![
        import_expansion(":first", "first"),
        import_expansion(":second", "second"),
    ];
    app.select(1);
    let selected_id = app.selected_id.clone().unwrap();
    app.config.expansion.swap(0, 1);

    assert_eq!(app.selected_index(), Some(0));
    assert_eq!(
        app.config.expansion[app.selected_index().unwrap()].id,
        selected_id
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn the_snippet_form_renders_every_field_kind_and_submits_with_enter() {
    let spec: crate::form::FormSpec = serde_json::from_str(
        r#"{"title":":tk","fields":[
            {"key":"field:name","label":"name","kind":{"text":{"default":"Ada"}}},
            {"key":"choice:Open|Resolved","label":"Choice","kind":{"choice":{"options":["Open","Resolved"]}}}
        ]}"#,
    )
    .unwrap();
    let outcome = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut app = crate::form::FormApp::new(
        spec,
        Palette::for_pack(ColorPack::Default, false),
        std::sync::Arc::clone(&outcome),
    );
    let ctx = egui::Context::default();
    let _ = run_gui_test_frame(&ctx, egui::RawInput::default(), |ui| app.render(ui));
    assert!(outcome.lock().unwrap().is_none());
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let _ = run_gui_test_frame(&ctx, input, |ui| app.render(ui));
    let values = outcome
        .lock()
        .unwrap()
        .clone()
        .expect("Enter submits the form");
    assert_eq!(values["field:name"], "Ada");
    assert_eq!(values["choice:Open|Resolved"], "Open");
}
