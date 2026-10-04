use super::storage::{parent_mode_is_secure, root_managed_parent_owner_allowed};
use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn safe_summary_does_not_echo_trigger_contents() {
    let error = ConfigError::DuplicateTrigger {
        trigger: "secret-trigger".into(),
        first: 1,
        second: 2,
    };
    let summary = error.safe_summary();
    assert!(summary.contains("1"));
    assert!(summary.contains("2"));
    assert!(!summary.contains("secret-trigger"));
}

#[test]
fn safe_summary_does_not_echo_untrusted_parent_path() {
    let error = ConfigError::InsecureParentOwner {
        path: "/home/user/private/secret-configs".into(),
        uid: 1234,
    };
    let summary = error.safe_summary();
    assert!(summary.contains("1234"));
    assert!(!summary.contains("secret-configs"));
}

#[test]
fn root_owned_world_writable_parent_mode_is_not_trusted() {
    // Ownership cannot make a directory safe when its mode grants write
    // access to group/other users; this is the regression behind the
    // parent-directory validation fix.
    assert!(!parent_mode_is_secure(0o0777));
    assert!(parent_mode_is_secure(0o1777));
    assert!(parent_mode_is_secure(0o0755));
}

#[test]
fn root_managed_configuration_requires_root_owned_parents() {
    assert!(root_managed_parent_owner_allowed(0));
    assert!(!root_managed_parent_owner_allowed(1000));
}

#[test]
fn generic_config_save_requires_admin_for_root_owned_target() {
    assert!(root_owned_target_requires_admin(0, 1000));
    assert!(!root_owned_target_requires_admin(0, 0));
    assert!(!root_owned_target_requires_admin(1000, 1000));
    assert!(!root_owned_target_requires_admin(1001, 1000));
}

#[test]
fn root_owned_config_below_user_owned_directory_is_rejected() {
    let uid = rustix::process::geteuid().as_raw();
    let parent = std::env::temp_dir().join(format!(
        "wayexpand-root-config-parent-{}-{}",
        std::process::id(),
        EXPANSION_ID_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&parent).unwrap();
    if uid != 0 {
        assert!(matches!(
            validate_root_managed_parent_chain(&parent.join("expansions.toml")),
            Err(ConfigError::InsecureParentOwner { uid: owner, .. }) if owner == uid
        ));
    }
    fs::remove_dir(parent).unwrap();
}

#[test]
fn propagate_case_uppercase_variant_colliding_with_another_trigger_is_rejected() {
    // `:sig` with propagate_case generates the matcher variant `:SIG`,
    // which collides with the second expansion's literal `:SIG`
    // trigger even though neither configured `trigger` string is a
    // literal duplicate of the other.
    let error = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        propagate_case = true

        [[expansion]]
        trigger = ":SIG"
        replacement = "something else"
        "#,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::DuplicateTrigger { .. }));
}

#[test]
fn propagate_case_capitalized_variant_colliding_with_another_trigger_is_rejected() {
    // `:sig` with propagate_case also generates `:Sig` (capitalized).
    let error = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        propagate_case = true

        [[expansion]]
        trigger = ":Sig"
        replacement = "something else"
        "#,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::DuplicateTrigger { .. }));
}

#[test]
fn propagate_case_without_collision_is_accepted() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        propagate_case = true

        [[expansion]]
        trigger = ":unrelated"
        replacement = "something else"
        "#,
    )
    .unwrap();
    assert_eq!(config.expansion.len(), 2);
}

#[test]
fn save_atomic_replaces_a_valid_configuration_and_keeps_private_mode() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-config-save-{}.toml", std::process::id()));
    fs::write(
        &path,
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"old\"\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut config = Config::load(&path).unwrap();
    config.expansion[0].replacement = "new".into();
    config.save_atomic(&path).unwrap();
    let reloaded = Config::load(&path).unwrap();
    assert_eq!(reloaded.expansion[0].replacement, "new");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn versioned_load_uses_one_source_snapshot_and_rejects_stale_save() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-revision-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let source =
        "# retained source snapshot\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"old\"\n";
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let loaded = Config::load_versioned(&path).unwrap();
    assert_ne!(loaded.source(), source);
    assert!(loaded.source().contains("id = "));
    let reloaded = Config::load_versioned(&path).unwrap();
    assert_eq!(
        loaded.config.expansion[0].id,
        reloaded.config.expansion[0].id
    );
    let mut stale_candidate = loaded.config.clone();
    stale_candidate.expansion[0].replacement = "stale edit".into();

    let mut external = loaded.config.clone();
    external.expansion[0].replacement = "external edit".into();
    external.save_atomic(&path).unwrap();
    let error = stale_candidate
        .save_atomic_if_revision_matches(&path, &loaded.revision)
        .unwrap_err();
    assert!(matches!(error, ConfigError::RevisionConflict));
    assert_eq!(
        Config::load(&path).unwrap().expansion[0].replacement,
        "external edit"
    );

    let filename = path.file_name().unwrap().to_string_lossy();
    let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn concurrent_conditional_writers_allow_only_one_revision_winner() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-concurrent-revision-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let initial =
        Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"initial\"\n").unwrap();
    initial.save_atomic(&path).unwrap();
    let loaded = Config::load_versioned(&path).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));

    let mut first = loaded.config.clone();
    first.expansion[0].replacement = "first writer".into();
    let first_barrier = barrier.clone();
    let first_path = path.clone();
    let first_revision = loaded.revision.clone();
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first
            .save_atomic_if_revision_matches(first_path, &first_revision)
            .is_ok()
    });

    let mut second = loaded.config;
    second.expansion[0].replacement = "second writer".into();
    let second_barrier = barrier;
    let second_path = path.clone();
    let second_revision = loaded.revision;
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second
            .save_atomic_if_revision_matches(second_path, &second_revision)
            .is_ok()
    });

    assert_ne!(first.join().unwrap(), second.join().unwrap());
    let saved = Config::load(&path).unwrap();
    assert!(matches!(
        saved.expansion[0].replacement.as_str(),
        "first writer" | "second writer"
    ));
    let filename = path.file_name().unwrap().to_string_lossy();
    let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn pre_category_config_without_new_fields_still_parses() {
    // A config written before `category`/`app_filter` existed: neither
    // field is present. `#[serde(default)]` must keep this loadable
    // indefinitely -- an old config file must never fail to parse just
    // because the schema grew new optional fields.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":legacy"
        replacement = "still works"
        description = "written before category/app_filter existed"
        tags = ["old"]
        match_mode = "immediate"
        enabled = true
        "#,
    )
    .unwrap();
    let expansion = &config.expansion[0];
    assert_eq!(expansion.trigger, ":legacy");
    assert_eq!(expansion.category, "");
    assert!(expansion.app_filter.is_empty());
    assert!(is_uuid(&expansion.id));
}

#[test]
fn generated_expansion_ids_are_unique_and_round_trip() {
    let config = Config::parse(
        "[[expansion]]\ntrigger=':one'\nreplacement='one'\n[[expansion]]\ntrigger=':two'\nreplacement='two'\n",
    )
    .unwrap();
    let first = config.expansion[0].id.clone();
    let second = config.expansion[1].id.clone();
    assert!(is_uuid(&first));
    assert_ne!(first, second);

    let encoded = toml::to_string(&config).unwrap();
    let decoded = Config::parse(&encoded).unwrap();
    assert_eq!(decoded.expansion[0].id, first);
    assert_eq!(decoded.expansion[1].id, second);
}

#[test]
fn expansion_ids_must_be_valid_and_unique() {
    let invalid =
        Config::parse("[[expansion]]\nid='not-a-uuid'\ntrigger=':one'\nreplacement='one'\n")
            .unwrap_err();
    assert!(matches!(
        invalid,
        ConfigError::InvalidExpansionId { index: 0 }
    ));

    let repeated = "[[expansion]]\nid='00000000-0000-4000-8000-000000000001'\ntrigger=':one'\nreplacement='one'\n[[expansion]]\nid='00000000-0000-4000-8000-000000000001'\ntrigger=':two'\nreplacement='two'\n";
    assert!(matches!(
        Config::parse(repeated),
        Err(ConfigError::DuplicateExpansionId {
            first: 0,
            second: 1
        })
    ));
}

#[test]
fn app_filter_rejects_empty_entries_and_excess_count() {
    let empty_entry = Config::parse(
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [\"\"]\n",
    );
    assert!(matches!(
        empty_entry,
        Err(ConfigError::InvalidAppFilter { index: 0 })
    ));

    let too_many = format!(
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"y\"\napp_filter = [{}]\n",
        (0..MAX_APP_FILTERS + 1)
            .map(|n| format!("\"app{n}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(matches!(
        Config::parse(&too_many),
        Err(ConfigError::InvalidAppFilter { index: 0 })
    ));
}

#[test]
fn app_filter_operators_and_safe_mode_are_explicit() {
    let exact = Config::parse(
        "[[expansion]]\ntrigger = ':x'\nreplacement = 'y'\napp_filter = ['app_id_exact:org.example.Editor']\n",
    )
    .unwrap();
    assert_eq!(
        exact.expansion[0].app_filter[0],
        "app_id_exact:org.example.Editor"
    );

    let weak = Config::parse(
        "[organization]\nsafe_mode = true\n[[expansion]]\ntrigger = ':x'\nreplacement = 'y'\napp_filter = ['app_id_glob:*editor*']\n",
    );
    assert!(matches!(
        weak,
        Err(ConfigError::InvalidAppFilter { index: 0 })
    ));

    let override_config = Config::parse(
        "[organization]\nsafe_mode = true\nallow_weak_app_filters = true\n[[expansion]]\ntrigger = ':x'\nreplacement = 'y'\napp_filter = ['title_contains:editor']\n",
    );
    assert!(override_config.is_ok());
}

#[test]
fn organization_policy_can_require_absolute_command_paths() {
    let error = Config::parse(
        r#"
        [organization]
        safe_mode = true
        require_absolute_commands = true

        [[expansion]]
        trigger = ":git"
        replacement = ""
        command = { program = "git", args = ["status"] }
        "#,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidCommand { index: 0, .. }
    ));

    let config = Config::parse(
        r#"
        [organization]
        safe_mode = true
        require_absolute_commands = true

        [[expansion]]
        trigger = ":git"
        replacement = ""
        command = { program = "/usr/bin/git", args = ["status"] }
        "#,
    )
    .unwrap();
    assert_eq!(
        config.expansion[0].command.as_ref().unwrap().program,
        "/usr/bin/git"
    );
}

#[test]
fn expansion_rejects_empty_named_action_ids() {
    let error = Config::parse(
        r#"
        [[expansion]]
        trigger = ":cluster"
        replacement = ""
        command = { action = "" }
        "#,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::InvalidCommand { index: 0, .. }
    ));
}

#[test]
fn organization_policy_can_require_absolute_hotkey_command_paths() {
    let error = Config::parse(
        r#"
        [organization]
        safe_mode = true
        require_absolute_commands = true

        [[hotkey]]
        chord = "Ctrl+Alt+T"
        command = { program = "konsole" }
        "#,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::InvalidHotkey { index: 0, .. }));
}

#[test]
fn undo_chord_cannot_collide_with_an_enabled_hotkey() {
    let error = Config::parse(
        r#"
        [settings]
        undo_chord = "control + z"

        [[hotkey]]
        chord = "Ctrl+Z"
        command = { program = "/bin/true" }
        "#,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::UndoHotkeyCollision { index: 0, chord } if chord == "Ctrl+Z"
    ));
}

#[test]
fn disabled_hotkey_may_match_the_undo_chord() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[hotkey]]
        enabled = false
        chord = "control+z"
        command = { program = "/bin/true" }
        "#,
    )
    .unwrap();

    assert!(!config.hotkey[0].enabled);
}

#[test]
fn audit_policy_allows_relative_command_paths() {
    let config = Config::parse(
        r#"
        [organization]
        safe_mode = false
        require_absolute_commands = true

        [[expansion]]
        trigger = ":git"
        replacement = ""
        command = { program = "git", args = ["status"] }

        [[hotkey]]
        chord = "Ctrl+Alt+T"
        command = { program = "konsole" }
        "#,
    )
    .unwrap();
    assert!(config.organization.require_absolute_commands);
}

#[test]
fn command_path_violation_blocks_only_in_safe_mode() {
    let audit = OrganizationPolicy {
        safe_mode: false,
        require_absolute_commands: true,
        ..OrganizationPolicy::default()
    };
    assert!(audit.command_path_violation("git").is_some());
    assert!(!audit.command_path_is_blocked("git"));

    let safe = OrganizationPolicy {
        safe_mode: true,
        ..audit.clone()
    };
    assert!(safe.command_path_violation("git").is_some());
    assert!(safe.command_path_is_blocked("git"));
    assert!(!safe.command_path_is_blocked("/usr/bin/git"));
}

#[test]
fn capability_requirements_distinguish_atomic_and_sensitive_guarantees() {
    let policy = OrganizationPolicy {
        safe_mode: true,
        require_atomic_replace: true,
        require_sensitive_focus: true,
        ..OrganizationPolicy::default()
    };
    let conservative = crate::InjectorCapabilities::default();
    assert_eq!(
        policy.capability_violation(conservative, false).as_deref(),
        Some("selected injector cannot guarantee atomic replacement transactions")
    );

    let atomic = crate::InjectorCapabilities {
        atomic_replace: true,
        ..crate::InjectorCapabilities::default()
    };
    assert_eq!(
        policy.capability_violation(atomic, false).as_deref(),
        Some("selected input source cannot report password or sensitive-field focus")
    );
    assert!(policy.capability_violation(atomic, true).is_none());
    assert!(policy
        .capability_violation_for_source(
            atomic,
            crate::InputSourceCapabilities {
                sensitive_focus: true,
                exclusive_capture: true,
                ..crate::InputSourceCapabilities::default()
            }
        )
        .is_none());
    assert!(policy
        .capability_violation_for_source(
            atomic,
            crate::InputSourceCapabilities {
                exclusive_capture: true,
                ..crate::InputSourceCapabilities::default()
            }
        )
        .is_some_and(|violation| violation.contains("sensitive-field focus")));
}

#[test]
fn audit_policy_has_no_effective_enforcement_values() {
    let policy = OrganizationPolicy {
        safe_mode: false,
        disable_commands: true,
        disable_hotkeys: true,
        require_absolute_commands: true,
        disable_title_matching: true,
        max_replacement_size: 256,
        allowed_backends: vec!["none".into()],
        allowed_packs: vec!["managed".into()],
        ..OrganizationPolicy::default()
    };
    let effective = policy.effective_enforcement_policy();

    assert!(!effective.disable_commands);
    assert!(!effective.disable_hotkeys);
    assert!(!effective.require_absolute_commands);
    assert!(!effective.disable_title_matching);
    assert!(!effective.require_atomic_replace);
    assert!(!effective.require_sensitive_focus);
    assert_eq!(effective.max_replacement_size, 0);
    assert!(effective.allowed_backends.is_empty());
    assert!(effective.allowed_packs.is_empty());
    assert!(policy
        .expansion_policy_violation(512, false, "wayland")
        .is_some());
}

#[test]
fn applying_audit_policy_keeps_engine_limits_unrestricted() {
    let mut config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":ok"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let policy = OrganizationPolicy {
        safe_mode: false,
        max_replacement_size: 256,
        ..OrganizationPolicy::default()
    };

    config.apply_administrator_policy(&policy).unwrap();
    assert_eq!(config.organization.max_replacement_size, 0);
}

#[test]
fn save_atomic_creates_missing_private_file() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-create-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config {
        expansion: Vec::new(),
        hotkey: Vec::new(),
        settings: Settings::default(),
        organization: OrganizationPolicy::default(),
    };
    config.save_atomic(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap().expansion.len(), 0);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(path).unwrap();
}
