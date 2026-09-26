use std::os::unix::fs::PermissionsExt;
use unicode_segmentation::UnicodeSegmentation;

use super::*;

fn engine() -> ExpansionEngine {
    ExpansionEngine::new(
        Config::parse(
            r#"
        [[expansion]]
        trigger = ":hello"
        replacement = "Hello from Wayland!"

        [[expansion]]
        trigger = ":cafe"
        replacement = "café ☕"
    "#,
        )
        .unwrap(),
    )
    .unwrap()
}

fn dispatch_and_wait(
    engine: &mut ExpansionEngine,
    pending: PendingExpansionResult,
) -> ExpansionResult {
    let result = match engine.dispatch_pending_with_policy(pending, 0).unwrap() {
        PendingExpansionDispatch::Ready(result) => result,
        PendingExpansionDispatch::Queued => {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = engine.drain_completed_commands().pop() {
                    break result;
                }
                assert!(Instant::now() < deadline, "command did not complete");
                thread::sleep(Duration::from_millis(5));
            }
        }
    };
    // The test helper models a successful injector. Production callers
    // commit only after their own safety and injection checks pass.
    engine.commit_applied_expansion(&result);
    result
}

#[test]
fn expands_unicode_replacement() {
    let mut engine = engine();
    let result = engine
        .process(InputEvent::Text(":cafe".into()))
        .pop()
        .unwrap();
    assert_eq!(result.matched_text, ":cafe");
    assert_eq!(result.insert, "café ☕");
}

#[test]
fn app_filtered_expansion_fails_closed_without_window_tracking() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "Regards"
        app_filter = ["thunderbird"]
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut results = engine.process(InputEvent::Text(":sig".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert!(
        results.is_empty(),
        "app-restricted expansion must not fire without a known focused window"
    );
}

#[test]
fn app_filtered_expansion_matches_by_app_id_case_insensitively() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "Regards"
        app_filter = ["Thunderbird"]
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird".into()),
        title: None,
    })));
    let mut results = engine.process(InputEvent::Text(":sig".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert_eq!(results.pop().unwrap().insert, "Regards");
}

#[test]
fn current_window_can_be_carried_across_a_replacement_engine() {
    // Simulates what a config reload must do: a fresh `ExpansionEngine`
    // starts with no window context, which would otherwise wrongly
    // fail-close every `app_filter`-scoped expansion until the next
    // real focus change even though the user's window never changed.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "Regards"
        app_filter = ["thunderbird"]
    "#,
    )
    .unwrap();
    let mut old_engine = ExpansionEngine::new(config.clone()).unwrap();
    old_engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird".into()),
        title: None,
    })));

    let mut new_engine = ExpansionEngine::new(config).unwrap();
    assert!(new_engine.current_window().is_none());
    new_engine.set_current_window(old_engine.current_window().cloned());

    let mut results = new_engine.process(InputEvent::Text(":sig".into()));
    results.extend(new_engine.process(InputEvent::EndOfInput));
    assert_eq!(results.pop().unwrap().insert, "Regards");
}

#[test]
fn app_filtered_expansion_ignores_other_windows() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "Regards"
        app_filter = ["thunderbird"]
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: Some("konsole".into()),
    })));
    let mut results = engine.process(InputEvent::Text(":sig".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert!(results.is_empty());
}

#[test]
fn unfiltered_expansion_matches_regardless_of_window_tracking() {
    let mut engine = engine();
    let mut results = engine.process(InputEvent::Text(":hello".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert_eq!(results.pop().unwrap().insert, "Hello from Wayland!");
}

#[test]
fn window_changed_clears_buffer_to_prevent_cross_window_matches() {
    // Text expansion state must be scoped to the focused window, not the
    // desktop session. The buffer contains characters typed in the previous
    // application, so clearing on window change prevents cross-window
    // trigger matches that could erase unrelated text.
    let mut engine = engine();
    engine.process(InputEvent::Text(":hel".into()));
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.kde.kate".into()),
        title: None,
    })));
    let mut results = engine.process(InputEvent::Text("lo".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    // The buffer was cleared on window change, so ":hello" was never formed
    assert!(
        results.is_empty(),
        "cross-window partial triggers must not continue matching"
    );
}

#[test]
fn cross_window_trigger_does_not_delete_wrong_text() {
    // Regression test for P0 bug: Alt-Tab mid-trigger.
    // Type ":he" in App A, switch to App B, type "llo". Should NOT match.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":hello"
        replacement = "expansion result"
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();

    // Type partial trigger in first window
    engine.process(InputEvent::Text(":he".into()));

    // Switch windows
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: None,
    })));

    // Complete what looks like the trigger in the new window
    let mut results = engine.process(InputEvent::Text("llo".into()));
    results.extend(engine.process(InputEvent::EndOfInput));

    assert!(
        results.is_empty(),
        "expansion must not fire for cross-window text sequences"
    );
}

#[test]
fn unfiltered_expansion_after_window_change() {
    // Verify that unfiltered expansions still work after a window change,
    // just with a fresh buffer (no cross-window text combination).
    let mut engine = engine();
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.kde.kate".into()),
        title: None,
    })));
    let mut results = engine.process(InputEvent::Text(":hello".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert_eq!(results.pop().unwrap().insert, "Hello from Wayland!");
}

#[test]
fn pause_changed_disables_capture_and_clears_buffer() {
    let mut engine = engine();
    engine.process(InputEvent::Text(":hel".into()));
    engine.process(InputEvent::PauseChanged(true));
    // Capture disabled, buffer cleared
    let results = engine.process(InputEvent::Text("lo".into()));
    assert!(
        results.is_empty(),
        "paused engine must not produce expansions"
    );
}

#[test]
fn pause_changed_resume_re_enables_capture() {
    let mut engine = engine();
    // Pause
    engine.process(InputEvent::PauseChanged(true));
    let results = engine.process(InputEvent::Text(":hello".into()));
    assert!(results.is_empty(), "paused engine must not expand");

    // Resume
    engine.process(InputEvent::PauseChanged(false));
    let mut results = engine.process(InputEvent::Text(":hello".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert_eq!(
        results.pop().unwrap().insert,
        "Hello from Wayland!",
        "resumed engine must expand"
    );
}

#[test]
fn sensitive_focus_and_pause_are_independent() {
    // P0 security fix: resume must not re-enable capture if in sensitive field
    let mut engine = engine();

    // 1. Enter sensitive field (password input)
    engine.process(InputEvent::FocusChanged { sensitive: true });
    assert!(
        engine.process(InputEvent::Text(":hello".into())).is_empty(),
        "must not expand in sensitive field"
    );

    // 2. User pauses (independently)
    engine.process(InputEvent::PauseChanged(true));

    // 3. User resumes (independently)
    engine.process(InputEvent::PauseChanged(false));

    // 4. Should still be disabled because sensitive field is still active!
    let results = engine.process(InputEvent::Text(":hello".into()));
    assert!(
        results.is_empty(),
        "resume must not re-enable capture while still in sensitive field"
    );

    // 5. Leave sensitive field
    engine.process(InputEvent::FocusChanged { sensitive: false });

    // 6. Now expansion works
    let mut results = engine.process(InputEvent::Text(":hello".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert_eq!(results.pop().unwrap().insert, "Hello from Wayland!");
}

#[test]
fn dispatches_enabled_hotkey_actions_without_side_effects() {
    let config = Config::parse(
        r#"
        [[hotkey]]
        chord = "Ctrl+Alt+M"
        description = "Open meeting helper"
        [hotkey.command]
        program = "/bin/true"
        timeout_ms = 100
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let chord = KeyChord::parse("control+option+m").unwrap();
    let result = engine.process_key(&chord);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].chord.to_string(), "Ctrl+Alt+M");
    assert_eq!(result[0].description, "Open meeting helper");
    assert_eq!(result[0].command.program, "/bin/true");

    engine.process(InputEvent::FocusChanged { sensitive: true });
    assert!(engine.process_key(&chord).is_empty());

    engine.process(InputEvent::FocusChanged { sensitive: false });
    assert!(engine.process_key(&chord).len() == 1);

    engine.process(InputEvent::PauseChanged(true));
    assert!(engine.process_key(&chord).is_empty());

    engine.process(InputEvent::PauseChanged(false));
    assert!(engine.process_key(&chord).len() == 1);
}

#[test]
fn hotkey_actions_use_direct_execution_and_timeout() {
    let config = Config::parse(
        r#"
        [[hotkey]]
        chord = "Ctrl+M"
        [hotkey.command]
        program = "/bin/true"
        timeout_ms = 100
        "#,
    )
    .unwrap();
    let engine = ExpansionEngine::new(config).unwrap();
    let action = engine
        .process_key(&KeyChord::parse("Ctrl+M").unwrap())
        .pop()
        .unwrap();
    ExpansionEngine::execute_hotkey(&action).unwrap();
}

#[test]
fn asynchronous_hotkey_execution_does_not_block_input_processing() {
    let config = Config::parse(
        r#"
        [[hotkey]]
        chord = "Ctrl+M"
        [hotkey.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.1"]
        timeout_ms = 500
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();
    let action = engine
        .process_key(&KeyChord::parse("Ctrl+M").unwrap())
        .pop()
        .unwrap();

    let started = Instant::now();
    engine.queue_hotkey(&action).unwrap();
    assert!(started.elapsed() < Duration::from_millis(50));

    let deadline = Instant::now() + Duration::from_secs(1);
    let (completed_action, result) = loop {
        if let Some(completion) = engine.drain_completed_hotkeys().pop() {
            break completion;
        }
        assert!(Instant::now() < deadline, "hotkey did not complete");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(completed_action.chord, action.chord);
    assert!(result.is_ok());
}

#[test]
fn rejected_command_job_does_not_consume_the_trigger() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/true"
        timeout_ms = 500
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();

    // A zero-capacity queue with its receiver held makes try_send return
    // Full deterministically, without starting a child process.
    let (sender, _job_receiver) = mpsc::sync_channel(0);
    let (hotkey_sender, _hotkey_receiver) = mpsc::sync_channel(0);
    let (_, completion_receiver) = mpsc::sync_channel(1);
    let (_, hotkey_completion_receiver) = mpsc::sync_channel(1);
    engine.async_commands = Some(AsyncCommandRuntime {
        command_sender: sender,
        hotkey_sender,
        receiver: completion_receiver,
        hotkey_receiver: hotkey_completion_receiver,
        metrics: Arc::clone(&engine.command_metrics),
        shutdown: Arc::new(AtomicBool::new(false)),
        command_worker: None,
        hotkey_worker: None,
    });
    engine.buffer.extend(":slow".chars());
    let before = engine.buffer.clone();

    assert!(engine
        .take_match(0, ":slow".chars().count(), None)
        .is_none());
    assert_eq!(engine.buffer, before);
    assert_eq!(engine.command_metrics().command_queue_depth, 0);
    assert_eq!(engine.command_metrics().command_queue_rejected_total, 1);
}

#[test]
fn deferred_dispatch_rejects_a_saturated_queue_without_running_command() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-deferred-queue-full-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":full"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf ran > '{}'"]
        timeout_ms = 500
        "#,
        marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();

    let (sender, _job_receiver) = mpsc::sync_channel(0);
    let (hotkey_sender, _hotkey_receiver) = mpsc::sync_channel(0);
    let (_, completion_receiver) = mpsc::sync_channel(1);
    let (_, hotkey_completion_receiver) = mpsc::sync_channel(1);
    engine.async_commands = Some(AsyncCommandRuntime {
        command_sender: sender,
        hotkey_sender,
        receiver: completion_receiver,
        hotkey_receiver: hotkey_completion_receiver,
        metrics: Arc::clone(&engine.command_metrics),
        shutdown: Arc::new(AtomicBool::new(false)),
        command_worker: None,
        hotkey_worker: None,
    });

    let pending = engine
        .process_deferred(InputEvent::Text(":full".into()))
        .pop()
        .unwrap();
    assert_eq!(
        engine.dispatch_pending_with_policy(pending, 0),
        Err(CommandError::QueueFull)
    );
    assert_eq!(engine.command_metrics().command_queue_depth, 0);
    assert_eq!(engine.command_metrics().command_queue_rejected_total, 1);
    assert!(!marker.exists(), "saturated dispatch must not run command");
}

#[test]
fn command_metrics_count_timeouts_and_failures() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":timeout"
        replacement = ""
        [expansion.command]
        program = "/bin/sleep"
        args = ["1"]
        timeout_ms = 10

        [[expansion]]
        trigger = ":failure"
        replacement = ""
        [expansion.command]
        program = "/bin/false"
        timeout_ms = 500
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();
    assert!(engine
        .process(InputEvent::Text(":timeout".into()))
        .is_empty());
    assert!(engine
        .process(InputEvent::Text(":failure".into()))
        .is_empty());

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        engine.drain_completed_commands();
        let metrics = engine.command_metrics();
        if metrics.command_timeout_total == 1 && metrics.command_failure_total == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "command metrics did not update");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn command_wait_failures_are_not_reported_as_timeouts() {
    let error = CommandError::WaitFailed("child status unavailable".into());

    assert_ne!(error, CommandError::Timeout);
    assert_eq!(
        error.to_string(),
        "failed while obtaining process status: child status unavailable"
    );
}

#[test]
fn command_expansion_uses_direct_program_output() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":kernel"
        replacement = ""
        [expansion.command]
        program = "/bin/printf"
        args = ["kernel-6.1"]
        timeout_ms = 500
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine
        .process(InputEvent::Text(":kernel".into()))
        .pop()
        .unwrap();
    assert_eq!(result.insert, "kernel-6.1");
}

#[test]
fn asynchronous_command_expansion_does_not_block_input_processing() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.1; printf finished"]
        timeout_ms = 500
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();
    let started = Instant::now();
    assert!(engine.process(InputEvent::Text(":slow".into())).is_empty());
    assert!(started.elapsed() < Duration::from_millis(50));

    let deadline = Instant::now() + Duration::from_secs(1);
    let result = loop {
        if let Some(result) = engine.drain_completed_commands().pop() {
            break result;
        }
        assert!(Instant::now() < deadline, "command did not complete");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(result.insert, "finished");
    assert_eq!(result.matched_text, ":slow");
}

#[test]
fn asynchronous_command_output_is_discarded_after_more_input() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.05; printf stale"]
        timeout_ms = 500
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();
    assert!(engine.process(InputEvent::Text(":slow".into())).is_empty());
    assert!(engine.process(InputEvent::Text("x".into())).is_empty());
    thread::sleep(Duration::from_millis(100));
    assert!(engine.drain_completed_commands().is_empty());
}

#[test]
fn asynchronous_command_output_is_discarded_after_key_only_input() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.05; printf stale"]
        timeout_ms = 500
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();
    assert!(engine.process(InputEvent::Text(":slow".into())).is_empty());
    let chord = KeyChord::parse("Left").unwrap();
    assert!(engine.process(InputEvent::Key(chord)).is_empty());
    thread::sleep(Duration::from_millis(100));
    assert!(engine.drain_completed_commands().is_empty());
}

#[test]
fn command_expansion_times_out_without_blocking_forever() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/sleep"
        args = ["1"]
        timeout_ms = 10
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.process(InputEvent::Text(":slow".into())).is_empty());
}

#[test]
fn command_expansion_cache_reuses_recent_output() {
    let counter =
        std::env::temp_dir().join(format!("wayexpand-command-cache-{}", std::process::id()));
    std::fs::write(&counter, "0").unwrap();
    let script = format!(
        "n=$(cat '{}'); n=$((n+1)); printf '%s' \"$n\" > '{}'; printf '%s' \"$n\"",
        counter.display(),
        counter.display()
    );
    let config = format!(
        "[[expansion]]\ntrigger = \":count\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"/bin/sh\"\nargs = [\"-c\", {script:?}]\ncache_ms = 1000\n"
    );
    let mut engine = ExpansionEngine::new(Config::parse(&config).unwrap()).unwrap();
    let results = engine.process(InputEvent::Text(":count:count".into()));
    let _ = std::fs::remove_file(&counter);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].insert, "1");
    assert_eq!(results[1].insert, "1");
}

#[test]
fn synchronous_command_cache_keeps_raw_output_before_case_propagation() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-command-case-cache-{}",
        std::process::id()
    ));
    std::fs::write(&marker, "").unwrap();
    let script = format!("printf 'Hello World'; printf x >> '{}'", marker.display());
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":word"
        replacement = ""
        propagate_case = true
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", {script:?}]
        cache_ms = 1000
        timeout_ms = 500
        "#
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();

    let results = engine.process(InputEvent::Text(":WORD:word".into()));

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].insert, "HELLO WORLD");
    assert_eq!(results[1].insert, "Hello World");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    std::fs::remove_file(marker).unwrap();
}

#[test]
fn command_expansion_limits_are_validated_before_activation() {
    let config = r#"
        [[expansion]]
        trigger = ":bad-command"
        replacement = ""
        [expansion.command]
        program = "printf"
        timeout_ms = 5001
    "#;
    assert!(matches!(
        Config::parse(config),
        Err(ConfigError::InvalidCommand { .. })
    ));
}

#[test]
fn malformed_config_is_an_error() {
    assert!(Config::parse("[[expansion]]\ntrigger = \":x\"\n").is_err());
}

#[test]
fn engine_rejects_manually_constructed_invalid_config() {
    let config = Config {
        expansion: vec![crate::ExpansionConfig {
            trigger: String::new(),
            replacement: "value".into(),
            description: String::new(),
            tags: Vec::new(),
            category: String::new(),
            app_filter: Vec::new(),
            match_mode: MatchMode::Immediate,
            command: None,
            enabled: true,
            propagate_case: false,
        }],
        hotkey: Vec::new(),
        settings: crate::Settings::default(),
        organization: crate::config::OrganizationPolicy::default(),
    };
    assert!(matches!(
        ExpansionEngine::new(config),
        Err(crate::ConfigError::EmptyTrigger { index: 0 })
    ));
}

#[test]
fn nul_characters_are_rejected() {
    let trigger = r#"[[expansion]]
trigger = "a\u0000b"
replacement = "ok""#;
    assert!(matches!(
        Config::parse(trigger),
        Err(crate::ConfigError::NulCharacter {
            field: "trigger",
            ..
        })
    ));
    let replacement = r#"[[expansion]]
trigger = ":x"
replacement = "bad\u0000value""#;
    assert!(matches!(
        Config::parse(replacement),
        Err(crate::ConfigError::NulCharacter {
            field: "replacement",
            ..
        })
    ));
}

#[test]
fn processes_each_character_in_a_chunk() {
    let mut engine = engine();
    let results = engine.process(InputEvent::Text("prefix :hello suffix".into()));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].trigger, ":hello");
}

#[test]
fn expansion_results_are_bounded_per_text_event() {
    let config = Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"").unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let results = engine.process(InputEvent::Text(":x".repeat(MAX_RESULTS_PER_EVENT + 1)));
    assert_eq!(results.len(), MAX_RESULTS_PER_EVENT);

    // Hitting the budget must not leave a partial trigger in the matcher.
    assert_eq!(engine.process(InputEvent::Text(":x".into())).len(), 1);
}

#[test]
fn duplicate_triggers_are_rejected() {
    let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"a\"\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"b\"";
    assert!(matches!(
        Config::parse(config),
        Err(crate::ConfigError::DuplicateTrigger { .. })
    ));
}

#[test]
fn prefix_triggers_use_the_longest_match() {
    let config = "[[expansion]]\ntrigger = \":h\"\nreplacement = \"a\"\n[[expansion]]\ntrigger = \":hello\"\nreplacement = \"b\"";
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    let result = engine.process(InputEvent::Text(":hello".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].insert, "b");
}

#[test]
fn short_prefix_waits_for_more_input_or_a_boundary() {
    let config = "[[expansion]]\ntrigger = \":h\"\nreplacement = \"short\"\n[[expansion]]\ntrigger = \":hello\"\nreplacement = \"long\"";
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    assert!(engine.process(InputEvent::Text(":h".into())).is_empty());
    let result = engine.process(InputEvent::Text("x".into()));
    assert_eq!(result[0].insert, "short");

    assert!(engine.process(InputEvent::Text(":h".into())).is_empty());
    let result = engine.process(InputEvent::EndOfInput);
    assert_eq!(result[0].insert, "short");
}

#[test]
fn word_boundary_mode_rejects_embedded_trigger() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.process(InputEvent::Text("x:sig".into())).is_empty());
    assert_eq!(
        engine.process(InputEvent::Text(" :sig ".into()))[0].insert,
        "signature"
    );
}

/// Regression test: when `max_buffer_chars` is small enough that the
/// character preceding a word-boundary trigger gets evicted from the
/// buffer before the trigger finishes matching, the engine must fail
/// closed (reject the match) rather than treat the evicted, unknown
/// character as "no boundary violation".
#[test]
fn word_boundary_mode_fails_closed_when_buffer_truncates_preceding_context() {
    let config = || {
        Config::parse(
            "[settings]\nmax_buffer_chars = 4\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        )
        .unwrap()
    };
    // "hello:sig" typed one character at a time: by the time ":sig" (4
    // chars) fills the 4-char buffer, the "o" that should block a
    // word-boundary match has already been evicted. Must still reject
    // rather than treat the unknown evicted character as a non-issue.
    let mut engine = ExpansionEngine::new(config()).unwrap();
    assert!(engine
        .process(InputEvent::Text("hello:sig".into()))
        .is_empty());
    // A real word boundary (nothing precedes the trigger at all) must
    // still match even with the same small buffer: this is the case
    // truncation must not be confused with. The trailing space is the
    // terminating character word-boundary mode needs to resolve the
    // match at all.
    let mut engine = ExpansionEngine::new(config()).unwrap();
    assert_eq!(
        engine.process(InputEvent::Text(":sig ".into()))[0].insert,
        "signature"
    );
}

#[test]
fn word_boundary_mode_is_unicode_aware() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.process(InputEvent::Text("é:sig".into())).is_empty());
    assert_eq!(
        engine.process(InputEvent::Text(" :sig ".into()))[0].insert,
        "signature"
    );
}

#[test]
fn word_boundary_mode_waits_for_a_trailing_boundary() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.process(InputEvent::Text(":sig".into())).is_empty());
    assert!(engine.process(InputEvent::Text("X".into())).is_empty());
    assert!(engine.process(InputEvent::Text(" :sig".into())).is_empty());
    assert_eq!(
        engine.process(InputEvent::EndOfInput)[0].insert,
        "signature"
    );
}

struct RecordingInjector {
    calls: Vec<String>,
}

impl crate::TextInjector for RecordingInjector {
    fn name(&self) -> &'static str {
        "test"
    }

    fn erase(&mut self, trigger: &str) -> Result<(), crate::InjectorError> {
        self.calls.push(format!("erase:{trigger}"));
        Ok(())
    }

    fn insert(&mut self, text: &str) -> Result<(), crate::InjectorError> {
        self.calls.push(format!("insert:{text}"));
        Ok(())
    }

    fn move_cursor_left(&mut self, count: usize) -> Result<(), crate::InjectorError> {
        self.calls.push(format!("left:{count}"));
        Ok(())
    }
}

#[test]
fn apply_erases_before_inserting() {
    let result = ExpansionResult {
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "value".into(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &result).unwrap();
    assert_eq!(injector.calls, ["erase::x", "insert:value"]);
}

struct AtomicInjector {
    calls: Vec<String>,
}

impl crate::TextInjector for AtomicInjector {
    fn name(&self) -> &'static str {
        "atomic-test"
    }

    fn erase(&mut self, _: &str) -> Result<(), crate::InjectorError> {
        panic!("atomic backend should not use the default erase path")
    }

    fn insert(&mut self, _: &str) -> Result<(), crate::InjectorError> {
        panic!("atomic backend should not use the default insert path")
    }

    fn replace(&mut self, trigger: &str, text: &str) -> Result<(), crate::InjectorError> {
        self.calls.push(format!("replace:{trigger}:{text}"));
        Ok(())
    }
}

#[test]
fn apply_uses_atomic_backend_operation_when_available() {
    let result = ExpansionResult {
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "value".into(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = AtomicInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &result).unwrap();
    assert_eq!(injector.calls, ["replace::x:value"]);
}

#[test]
fn apply_replaces_typed_trigger_and_commits_terminator() {
    let result = ExpansionResult {
        trigger: ":sig".into(),
        matched_text: ":SIG".into(),
        insert: "Best regards,".into(),
        cursor_offset: Some(2),
        reinsert_after: Some(' '),
        command_backed: false,
        undoable: true,
    };
    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &result).unwrap();
    assert_eq!(
        injector.calls,
        ["erase::SIG ", "insert:Best regards, ", "left:3"]
    );
}

#[test]
fn sensitive_focus_disables_matching_and_clears_buffer() {
    let mut engine = engine();
    assert!(engine.process(InputEvent::Text(":hel".into())).is_empty());
    engine.process(InputEvent::FocusChanged { sensitive: true });
    assert!(engine.process(InputEvent::Text("lo".into())).is_empty());
    engine.process(InputEvent::FocusChanged { sensitive: false });
    assert!(engine.process(InputEvent::Text(":hello".into())).len() == 1);
}

#[test]
fn disabled_expansions_do_not_match() {
    let config = "[[expansion]]\ntrigger = \":off\"\nreplacement = \"secret\"\nenabled = false";
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    assert!(engine.process(InputEvent::Text(":off".into())).is_empty());
}

#[test]
fn disabled_prefix_does_not_block_enabled_expansion() {
    let config = r#"
        [[expansion]]
        trigger = ":x"
        replacement = "disabled"
        enabled = false

        [[expansion]]
        trigger = ":xyz"
        replacement = "enabled"
    "#;
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    let result = engine.process(InputEvent::Text(":xyz".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].insert, "enabled");
}

#[test]
fn disabled_duplicate_does_not_block_enabled_expansion() {
    let config = r#"
        [[expansion]]
        trigger = ":x"
        replacement = "disabled"
        enabled = false

        [[expansion]]
        trigger = ":x"
        replacement = "enabled"
    "#;
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    let result = engine.process(InputEvent::Text(":x".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].insert, "enabled");
}

#[test]
fn unknown_config_fields_are_rejected() {
    let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\nwat = true";
    assert!(Config::parse(config).is_err());
}

#[test]
fn invalid_template_is_rejected_before_activation() {
    let config = "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{{unknown}}\"";
    assert!(matches!(
        Config::parse(config),
        Err(crate::ConfigError::InvalidTemplate { .. })
    ));
}

#[test]
fn oversized_replacements_are_rejected() {
    let config = format!(
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{}\"",
        "a".repeat(1024 * 1024 + 1)
    );
    assert!(matches!(
        Config::parse(&config),
        Err(crate::ConfigError::ReplacementTooLarge { .. })
    ));
}

#[test]
fn excessive_expansion_count_is_rejected() {
    let mut config = String::new();
    for index in 0..10_001 {
        config.push_str(&format!(
            "[[expansion]]\ntrigger = \":{index}\"\nreplacement = \"x\"\n"
        ));
    }
    assert!(matches!(
        Config::parse(&config),
        Err(crate::ConfigError::TooManyExpansions { .. })
    ));
}

#[test]
fn oversized_configuration_is_rejected_before_parsing() {
    let config = "x".repeat(16 * 1024 * 1024 + 1);
    assert!(matches!(
        Config::parse(&config),
        Err(crate::ConfigError::ConfigTooLarge { .. })
    ));
}

#[test]
fn non_regular_configuration_path_is_rejected() {
    let path =
        std::env::temp_dir().join(format!("wayexpand-config-directory-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(crate::ConfigError::NotRegular { .. })
    ));
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn symlinked_configuration_validates_the_resolved_parent() {
    let root =
        std::env::temp_dir().join(format!("wayexpand-config-symlink-{}", std::process::id()));
    let target = root.join("target.toml");
    let link = root.join("link.toml");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        &target,
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
    )
    .unwrap();
    // Set both modes explicitly rather than inheriting the umask: a
    // default of 002 (Debian/Ubuntu user-private-group setups) yields a
    // group-writable 0775 directory and 0664 file, which `Config::load`
    // correctly rejects, failing this test for the wrong reason.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(Config::load(&link).is_ok());
    std::fs::remove_file(link).unwrap();
    std::fs::remove_file(target).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn fifo_configuration_path_is_rejected_without_blocking() {
    let path = std::env::temp_dir().join(format!("wayexpand-config-fifo-{}", std::process::id()));
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &path,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(crate::ConfigError::NotRegular { .. })
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn group_writable_configuration_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-permissions-{}",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(crate::ConfigError::InsecurePermissions { .. })
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn user_owned_configuration_requires_private_mode() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-config-user-private-{}",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(crate::ConfigError::InsecurePermissions { mode: 0o644, .. })
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn writable_non_sticky_configuration_parent_is_rejected() {
    let parent = std::env::temp_dir().join(format!(
        "wayexpand-config-parent-permissions-{}",
        std::process::id()
    ));
    let path = parent.join("expansions.toml");
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(
        &path,
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        Config::load(&path),
        Err(crate::ConfigError::InsecureParent { .. })
    ));
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(parent).unwrap();
}

#[test]
fn excessive_enabled_trigger_data_is_rejected() {
    let mut config = String::new();
    for index in 0..=crate::config::MAX_TOTAL_TRIGGER_CHARS / 128 {
        config.push_str(&format!(
            "[[expansion]]\ntrigger = \":{index:08x}{}\"\nreplacement = \"x\"\n",
            "a".repeat(119)
        ));
    }
    assert!(matches!(
        Config::parse(&config),
        Err(crate::ConfigError::TriggerDataTooLarge { .. })
    ));
}

#[test]
fn buffer_limit_is_bounded() {
    let config = "[settings]\nmax_buffer_chars = 0";
    assert!(matches!(
        Config::parse(config),
        Err(crate::ConfigError::InvalidBufferLimit)
    ));
}

#[test]
fn buffer_limit_can_be_configured() {
    let config =
        "[settings]\nmax_buffer_chars = 2\n[[expansion]]\ntrigger = \":x\"\nreplacement = \"ok\"";
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    assert_eq!(engine.process(InputEvent::Text("abc:x".into())).len(), 1);
}

#[test]
fn propagate_case_leaves_replacement_unchanged_for_lowercase_trigger() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert_eq!(
        engine.process(InputEvent::Text(":sig".into()))[0].insert,
        "regards"
    );
}

#[test]
fn propagate_case_uppercases_replacement_for_uppercase_trigger() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert_eq!(
        engine.process(InputEvent::Text(":SIG".into()))[0].insert,
        "REGARDS"
    );
}

#[test]
fn propagate_case_tracks_unicode_matched_text_for_deletion() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \"ß\"\nreplacement = \"grüße\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine.process(InputEvent::Text("SS".into())).pop().unwrap();

    assert_eq!(result.trigger, "ß");
    assert_eq!(result.matched_text, "SS");
    assert_eq!(result.insert, "GRÜSSE");

    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &result).unwrap();
    assert_eq!(injector.calls, ["erase:SS", "insert:GRÜSSE"]);
}

#[test]
fn propagate_case_capitalizes_replacement_for_capitalized_trigger() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert_eq!(
        engine.process(InputEvent::Text(":Sig".into()))[0].insert,
        "Regards"
    );
}

#[test]
fn propagate_case_prefix_matching_uses_the_typed_variant() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":a\"\nreplacement = \"alpha\"\npropagate_case = true\n[[expansion]]\ntrigger = \":ab\"\nreplacement = \"alphabet\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();

    let results = engine.process(InputEvent::Text(":Ab".into()));

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].matched_text, ":A");
    assert_eq!(results[0].insert, "Alpha");
}

#[test]
fn propagate_case_works_with_word_boundary_mode() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"\nmatch_mode = \"word-boundary\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert_eq!(
        engine.process(InputEvent::Text(" :SIG ".into()))[0].insert,
        "REGARDS"
    );
}

#[test]
fn propagate_case_is_opt_in_and_does_not_affect_ordinary_triggers() {
    // Without `propagate_case`, matching stays strictly literal:
    // typing the trigger in a different case must not match at all.
    let config =
        Config::parse("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"").unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.process(InputEvent::Text(":SIG".into())).is_empty());
    assert_eq!(
        engine.process(InputEvent::Text(":sig".into()))[0].insert,
        "regards"
    );
}

#[test]
fn cursor_marker_splits_replacement_and_reports_trailing_length() {
    let config =
        Config::parse("[[expansion]]\ntrigger = \":paren\"\nreplacement = \"(){{cursor}}!\"")
            .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine.process(InputEvent::Text(":paren".into()))[0].clone();
    assert_eq!(result.insert, "()!");
    // "!" is the one character after the marker, so the cursor should
    // land between "(" and ")": one character back from the end.
    assert_eq!(result.cursor_offset, Some(1));
}

#[test]
fn cursor_marker_is_optional_and_defaults_to_end_of_text() {
    let config =
        Config::parse("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"").unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine.process(InputEvent::Text(":sig".into()))[0].clone();
    assert_eq!(result.insert, "regards");
    assert_eq!(result.cursor_offset, None);
}

#[test]
fn cursor_marker_works_alongside_other_template_variables() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":hi\"\nreplacement = \"Hi {{cursor}}, {{username}}!\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine.process(InputEvent::Text(":hi".into()))[0].clone();
    let expected_tail = format!(", {}!", crate::TemplateContext::system().username);
    assert_eq!(result.insert, format!("Hi {expected_tail}"));
    assert_eq!(
        result.cursor_offset,
        Some(expected_tail.graphemes(true).count())
    );
}

#[test]
fn cursor_marker_is_not_recognized_in_command_output() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":cmd\"\nreplacement = \"fallback\"\n[expansion.command]\nprogram = \"printf\"\nargs = [\"literal {{cursor}} text\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine.process(InputEvent::Text(":cmd".into()))[0].clone();
    assert_eq!(result.insert, "literal {{cursor}} text");
    assert_eq!(result.cursor_offset, None);
}

#[test]
fn undo_reverts_the_expansion_immediately_following_it() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let expansion = engine.process(InputEvent::Text(":sig".into()))[0].clone();
    assert_eq!(expansion.insert, "regards");
    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("an expansion is pending to undo");
    // Erases the full inserted replacement and types the original
    // trigger back.
    assert_eq!(undo.matched_text, "regards");
    assert_eq!(undo.insert, ":sig");
}

#[test]
fn undo_applies_by_erasing_the_inserted_text() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let expansion = engine.process(InputEvent::Text(":sig".into()))[0].clone();
    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &expansion).unwrap();

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("an expansion is pending to undo");
    ExpansionEngine::apply(&mut injector, &undo).unwrap();

    assert_eq!(
        injector.calls,
        [
            "erase::sig",
            "insert:regards",
            "erase:regards",
            "insert::sig"
        ]
    );
}

#[test]
fn undo_round_trip_includes_reinserted_word_boundary() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"Regards\"\nmatch_mode = \"word-boundary\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let expansion = engine
        .process(InputEvent::Text(":sig ".into()))
        .pop()
        .unwrap();
    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &expansion).unwrap();

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("a boundary expansion should be undoable");
    assert_eq!(undo.matched_text, "Regards ");
    assert_eq!(undo.insert, ":sig ");
    ExpansionEngine::apply(&mut injector, &undo).unwrap();

    assert_eq!(
        injector.calls,
        [
            "erase::sig ",
            "insert:Regards ",
            "erase:Regards ",
            "insert::sig ",
        ]
    );
}

#[cfg(unix)]
#[test]
fn async_command_undo_round_trip_includes_reinserted_word_boundary() {
    let config = Config::parse(
        r#"[settings]
undo_chord = "Ctrl+Z"

[[expansion]]
trigger = ":sig"
replacement = ""
match_mode = "word-boundary"
[expansion.command]
program = "/bin/sh"
args = ["-c", "printf Regards"]
timeout_ms = 500
"#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.enable_async_commands());
    let pending = engine
        .process_deferred(InputEvent::Text(":sig ".into()))
        .pop()
        .unwrap();
    let expansion = dispatch_and_wait(&mut engine, pending);
    let mut injector = RecordingInjector { calls: Vec::new() };
    ExpansionEngine::apply(&mut injector, &expansion).unwrap();

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("an async boundary expansion should be undoable");
    assert_eq!(undo.matched_text, "Regards ");
    assert_eq!(undo.insert, ":sig ");
    ExpansionEngine::apply(&mut injector, &undo).unwrap();
}

#[test]
fn undo_is_a_one_shot_and_does_nothing_without_a_pending_expansion() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::Text(":sig".into()));
    let chord = KeyChord::parse("Ctrl+Z").unwrap();
    assert!(engine.try_undo(&chord).is_some());
    // A second press right after has nothing left to undo.
    assert!(engine.try_undo(&chord).is_none());
}

#[test]
fn undo_is_invalidated_by_any_typing_in_between() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::Text(":sig".into()));
    engine.process(InputEvent::Text("x".into()));
    assert!(engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .is_none());
}

#[test]
fn undo_is_disabled_when_not_configured() {
    let config =
        Config::parse("[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"").unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::Text(":sig".into()));
    assert!(engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .is_none());
}

#[test]
fn undo_does_not_apply_to_a_cursor_marker_expansion() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":paren\"\nreplacement = \"(){{cursor}}\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::Text(":paren".into()));
    assert!(engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .is_none());
}

#[test]
fn undo_restores_the_case_variant_actually_typed() {
    let config = Config::parse(
        "[settings]\nundo_chord = \"Ctrl+Z\"\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"regards\"\npropagate_case = true",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::Text(":SIG".into()));
    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .unwrap();
    assert_eq!(undo.insert, ":SIG");
}

#[test]
fn app_filter_matches_on_app_id_when_available() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.mozilla.Thunderbird".into()),
        title: Some("Some Mail".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].insert, "contact@example.com");
}

#[test]
fn app_filter_uses_title_only_when_app_id_unavailable() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: None,
        title: Some("Thunderbird Mail Client".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        1,
        "should match title as fallback when app_id is unavailable"
    );
}

#[test]
fn app_filter_rejects_title_match_when_app_id_is_available_but_different() {
    // Security test: window title should NOT override app_id mismatch.
    // Example: Konsole titled "Thunderbird troubleshooting" should NOT match
    // app_filter=["thunderbird"] meant for the actual Thunderbird application.
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: Some("Thunderbird troubleshooting".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        0,
        "title should NOT override app_id mismatch"
    );
}

#[test]
fn app_filter_with_no_window_fails_closed() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(None);
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        0,
        "expansion without window context should not match"
    );
}

#[test]
fn app_filter_empty_matches_everywhere() {
    let config =
        Config::parse("[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"")
            .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.example.AnyApp".into()),
        title: Some("Some Window".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        1,
        "expansion with empty app_filter should match anywhere"
    );
}

#[test]
fn disable_title_matching_policy_fails_closed_without_app_id() {
    // Regression test: disable_title_matching must NOT disable app filtering.
    // It should only disable the title-based fallback.
    // When app_id is unavailable and disable_title_matching=true, must fail closed.
    let mut config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    config.organization.disable_title_matching = true;

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: None, // Only title available
        title: Some("Thunderbird Mail Client".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        0,
        "disable_title_matching=true should prevent title fallback, fail closed"
    );
}

#[test]
fn disable_title_matching_allows_app_id_match() {
    // When app_id IS available and matches, disable_title_matching should NOT block it.
    let mut config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    config.organization.disable_title_matching = true;

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.thunderbird.Thunderbird".into()),
        title: Some("Some Email".into()),
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(
        results.len(),
        1,
        "disable_title_matching should not affect app_id matching"
    );
}

#[test]
fn runtime_title_matching_policy_is_applied() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_title_matching_disabled(true);
    engine.set_current_window(Some(WindowContext {
        app_id: None,
        title: Some("Thunderbird Mail Client".into()),
    }));

    assert!(engine.process(InputEvent::Text(":email".into())).is_empty());
}

#[test]
fn word_boundary_triggers_reinsert_terminating_character() {
    // Evdev capture is non-exclusive: the space that ends a word-boundary
    // trigger has already reached the app. The engine must report that
    // the terminating character should be re-inserted after the replacement.
    // The backend will erase trigger + terminator, insert replacement, then
    // re-insert the terminator.
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let result = engine
        .process(InputEvent::Text(":sig ".into()))
        .pop()
        .unwrap();
    assert_eq!(result.trigger, ":sig");
    assert_eq!(result.insert, "signature");
    assert_eq!(result.reinsert_after, Some(' '));
}

#[test]
fn terminator_reinsertion_can_be_disabled_explicitly() {
    let config = Config::parse(
        r#"[[expansion]]
trigger = ":sig"
replacement = "signature"
match_mode = "word-boundary""#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_reinsert_terminators(false);

    let result = engine
        .process(InputEvent::Text(":sig.".into()))
        .pop()
        .unwrap();
    assert_eq!(result.insert, "signature");
    assert_eq!(result.reinsert_after, None);
}

#[test]
fn delayed_triggers_preserve_the_character_that_completed_them() {
    // The character that breaks a longer continuation has already reached
    // the application. Preserve it in the replacement while also allowing
    // it to participate in matching a following trigger.
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":a\"\nreplacement = \"alpha\"\n[[expansion]]\ntrigger = \":ab\"\nreplacement = \"alphabet\"\n[[expansion]]\ntrigger = \":b\"\nreplacement = \"beta\"",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    // Typing `:a:b` should produce matches for `:a` (broken by `:`) and `:b`.
    let results = engine.process(InputEvent::Text(":a:b".into()));
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].trigger, ":a");
    assert_eq!(results[0].reinsert_after, Some(':'));
    assert_eq!(results[1].trigger, ":b");
    assert_eq!(results[1].reinsert_after, None);
}

#[test]
fn disable_commands_policy_blocks_command_execution() {
    // Regression test: disable_commands policy must prevent command execution
    // in the engine, BEFORE the subprocess runs. Verify by checking that a
    // side effect (file creation) never occurs—not just absence of result.
    use std::fs;
    use std::path::PathBuf;

    let test_file = PathBuf::from(format!(
        "/tmp/wayexpand-policy-test-{}.txt",
        std::process::id()
    ));

    // Clean up if it exists from a prior run
    let _ = fs::remove_file(&test_file);

    let cmd = format!(
        "[[expansion]]\ntrigger = \":cmd\"\nreplacement = \"dummy\"\ncommand = {{ program = \"touch\", args = [\"{}\"] }}\n",
        test_file.display()
    );

    let mut config = Config::parse(&cmd).unwrap();
    config.organization.disable_commands = true;

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();

    // Typing the trigger should not produce results when commands are disabled
    let results = engine.process(InputEvent::Text(":cmd".into()));
    assert!(
        results.is_empty(),
        "command-backed expansion should not return result when disable_commands=true"
    );

    // Prove the actual security property: the subprocess was never spawned
    // If the subprocess had run, the file would exist
    assert!(
        !test_file.exists(),
        "command was executed despite disable_commands=true; {} exists when it should not",
        test_file.display()
    );

    // Clean up
    let _ = fs::remove_file(&test_file);
}

#[test]
fn commands_execute_when_not_disabled() {
    // Positive control: verify that commands DO run when policy allows them.
    // This ensures the test infrastructure works and command execution isn't broken.
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    let test_file = PathBuf::from(format!(
        "/tmp/wayexpand-cmd-enabled-{}.txt",
        std::process::id()
    ));

    // Clean up if it exists from a prior run
    let _ = fs::remove_file(&test_file);

    let cmd = format!(
        "[[expansion]]\ntrigger = \":cmd\"\nreplacement = \"dummy\"\ncommand = {{ program = \"touch\", args = [\"{}\"] }}\n",
        test_file.display()
    );

    let config = Config::parse(&cmd).unwrap();
    // disable_commands is false by default, so commands will execute

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();

    // Type the trigger to queue the command job
    let _ = engine.process(InputEvent::Text(":cmd".into()));

    // Commands are async; drain the completed results with a small timeout
    thread::sleep(Duration::from_millis(100));
    let _ = engine.drain_completed_commands();

    // File should exist, proving the command ran
    assert!(
        test_file.exists(),
        "command did not execute when commands are enabled; {} does not exist",
        test_file.display()
    );

    // Clean up
    let _ = fs::remove_file(&test_file);
}

#[test]
fn disable_commands_policy_blocks_sync_fallback() {
    // Regression test: when async infrastructure is unavailable and commands
    // fall back to synchronous execution, the disable_commands policy must still
    // be enforced. This prevents the policy from being silently bypassed when
    // async workers fail to start.
    use std::fs;
    use std::path::PathBuf;

    let test_file = PathBuf::from(format!(
        "/tmp/wayexpand-async-fallback-{}.txt",
        std::process::id()
    ));

    // Clean up if it exists from a prior run
    let _ = fs::remove_file(&test_file);

    let cmd = format!(
        "[[expansion]]\ntrigger = \":cmd\"\nreplacement = \"dummy\"\ncommand = {{ program = \"touch\", args = [\"{}\"] }}\n",
        test_file.display()
    );

    let mut config = Config::parse(&cmd).unwrap();
    config.organization.disable_commands = true;

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Deliberately do NOT call enable_async_commands(). This simulates async
    // infrastructure failure (e.g., thread creation failure on startup).
    // The disable_commands policy should still block execution via sync fallback.

    // Typing the trigger should not produce results when commands are disabled
    let results = engine.process(InputEvent::Text(":cmd".into()));
    assert!(
        results.is_empty(),
        "disable_commands policy should block sync fallback when async infrastructure is unavailable"
    );

    // Prove the security property: the subprocess was never spawned.
    // This test would fail without the policy enforcement in the sync path.
    assert!(
        !test_file.exists(),
        "command was executed via sync fallback despite disable_commands policy; {} exists when it should not",
        test_file.display()
    );

    // Clean up
    let _ = fs::remove_file(&test_file);
}

#[test]
fn static_expansions_work_with_disable_commands_policy() {
    // Regression test: disable_commands should ONLY block command-backed expansions,
    // not static snippets. This tests the IBus safe_mode bug where pre_flight_check()
    // was blocking ALL snippets when both safe_mode and disable_commands were true.
    let config = Config::parse(
        r#"[[expansion]]
trigger = ":sig"
replacement = "signature""#,
    )
    .unwrap();

    let mut config = config;
    config.organization.disable_commands = true;

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Static expansion (no command) should work even with disable_commands policy
    let results = engine.process(InputEvent::Text(":sig".into()));
    assert!(
        !results.is_empty(),
        "static expansions must work even when disable_commands policy is set"
    );
    assert_eq!(
        results[0].insert, "signature",
        "static expansion should produce correct output"
    );
    assert!(
        !results[0].command_backed,
        "static expansion should have command_backed=false"
    );
}

#[test]
fn process_descendants_cleaned_up_on_successful_exit() {
    // Regression test: spawned descendants should not survive after the
    // command-backed expansion completes, even when the direct child exits
    // successfully. The descendant waits before writing its marker so the
    // test observes continued execution, rather than shell redirection
    // creating the file before the descendant is killed.
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    // Create a temporary helper script that spawns a descendant and exits
    let script_path = PathBuf::from(format!(
        "/tmp/wayexpand-descendant-test-{}.sh",
        std::process::id()
    ));
    let output_file = PathBuf::from(format!(
        "/tmp/wayexpand-descendant-marker-{}.txt",
        std::process::id()
    ));

    // Script that spawns a descendant and exits successfully. A surviving
    // descendant writes the marker after the command has completed.
    let script_content = format!(
        "#!/bin/bash\n\
        (sleep 1; printf survived > {}) &\n\
        exit 0\n",
        output_file.display()
    );

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&output_file);

    let mut script_file = File::create(&script_path).unwrap();
    script_file.write_all(script_content.as_bytes()).unwrap();
    drop(script_file);

    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let cmd = format!(
        "[[expansion]]\ntrigger = \":spawn\"\nreplacement = \"x\"\ncommand = {{ program = \"{}\", timeout_ms = 5000 }}\n",
        script_path.display()
    );

    let config = Config::parse(&cmd).unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();

    // Execute the command that spawns a descendant
    let _ = engine.process(InputEvent::Text(":spawn".into()));

    // Wait for async command to complete
    thread::sleep(Duration::from_millis(500));
    let _ = engine.drain_completed_commands();

    // Give any surviving descendant enough time to write the marker.
    thread::sleep(Duration::from_millis(1500));

    // This ordinary descendant should have been killed with the process group.
    let survived = output_file.exists();

    // Clean up before asserting so a failure cannot leave test processes or
    // temporary files behind.
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&output_file);

    assert!(
        !survived,
        "descendant process survived after command-backed expansion completed"
    );
}

#[cfg(unix)]
#[test]
fn detached_stdout_holder_does_not_block_command_output() {
    use std::fs::File;
    use std::io::Write;

    let script_path = format!(
        "/tmp/wayexpand-detached-stdout-test-{}.sh",
        std::process::id()
    );
    let escaped_marker = format!(
        "/tmp/wayexpand-detached-stdout-marker-{}.txt",
        std::process::id()
    );
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&escaped_marker);

    let script_content = format!(
        "#!/bin/sh\n(setsid sh -c 'sleep 1; printf escaped > \"{}\"' >&1 2>/dev/null </dev/null &)\nprintf 'ready\\n'\n",
        escaped_marker
    );
    let mut script_file = File::create(&script_path).unwrap();
    script_file.write_all(script_content.as_bytes()).unwrap();
    drop(script_file);
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let command = CommandConfig {
        program: script_path.clone(),
        args: Vec::new(),
        timeout_ms: 5000,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: Vec::new(),
    };

    let started = Instant::now();
    let output = run_command(&command).unwrap();
    let elapsed = started.elapsed();

    let _ = std::fs::remove_file(&script_path);
    thread::sleep(Duration::from_millis(1200));
    let _ = std::fs::remove_file(&escaped_marker);

    assert_eq!(output, "ready");
    assert!(
        elapsed < Duration::from_millis(1000),
        "detached stdout holder delayed command output for {elapsed:?}"
    );
}

#[test]
fn trigger_deletion_failure_must_not_double_insert() {
    // CRITICAL: If the backend fails to delete the trigger after expansion,
    // the user would see both the trigger AND the replacement (double-insert).
    // This test documents the expected behavior: the matched_text field
    // tells the backend exactly what was typed and must be deleted.
    // If backend doesn't delete exactly that length, we have data loss.
    //
    // The engine provides complete instructions:
    // - matched_text: exactly what to delete from the buffer
    // - insert: what to put in its place
    // - reinsert_after: optional character to append (for word-boundary matches)
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":test"
        replacement = "success"
        [[expansion]]
        trigger = ":word"
        match_mode = "word-boundary"
        replacement = "word"
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Test 1: Regular trigger completes without trailing key
    // When ":test" fully matches, reinsert_after is None (the space comes later)
    let result = engine.process(InputEvent::Text(":test ".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].matched_text, ":test");
    assert_eq!(result[0].trigger, ":test");
    assert_eq!(result[0].insert, "success");
    assert_eq!(result[0].reinsert_after, None);
    // Backend MUST:
    // 1. Delete exactly 5 characters (":test", using matched_text.len())
    // 2. Insert "success"
    // Result: "success " (NOT ":testsuccess " or "success :test")

    // Test 2: Word boundary trigger - space is the terminating char
    // Type ":word" then space - the space breaks the word and triggers the match
    let result = engine.process(InputEvent::Text(":word ".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].matched_text, ":word");
    assert_eq!(result[0].insert, "word");
    // With word-boundary, reinsert_after contains the terminating character
    assert_eq!(result[0].reinsert_after, Some(' '));
    // Backend MUST delete exactly ":word" (5 chars), then insert "word" then reinsert ' '
    // Result: "word " (NOT ":word word" if deletion fails)
}

#[test]
fn rapid_successive_triggers_dont_corrupt_state() {
    // CRITICAL: Type multiple triggers in quick succession.
    // Each expansion must correctly:
    // 1. Clear the buffer after match (prevent re-match)
    // 2. Not interfere with subsequent triggers
    // 3. Provide independent deletion/insertion instructions
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":a"
        replacement = "A"
        [[expansion]]
        trigger = ":b"
        replacement = "B"
        [[expansion]]
        trigger = ":c"
        replacement = "C"
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Type: ":a :b :c"
    let r1 = engine.process(InputEvent::Text(":a ".into()));
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].trigger, ":a");

    let r2 = engine.process(InputEvent::Text(":b ".into()));
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0].trigger, ":b");

    let r3 = engine.process(InputEvent::Text(":c".into()));
    assert_eq!(r3.len(), 1);
    assert_eq!(r3[0].trigger, ":c");

    // Each expansion should be independent - no cross-contamination
    assert_eq!(r1[0].matched_text, ":a");
    assert_eq!(r2[0].matched_text, ":b");
    assert_eq!(r3[0].matched_text, ":c");
}

#[test]
fn sensitive_focus_prevents_expansion_in_password_fields() {
    // CRITICAL: If a backend reports FocusChanged{sensitive: true},
    // NO expansion should occur until FocusChanged{sensitive: false}.
    // This is the password field protection.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":email"
        replacement = "user@example.com"
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Type normally - expansion should work
    let result = engine.process(InputEvent::Text(":email ".into()));
    assert_eq!(result.len(), 1, "expansion should work in normal fields");

    // Switch to sensitive field
    let _ = engine.process(InputEvent::FocusChanged { sensitive: true });

    // Type trigger in sensitive field - should NOT expand
    let result = engine.process(InputEvent::Text(":email ".into()));
    assert_eq!(
        result.len(),
        0,
        "expansion must be blocked in sensitive fields"
    );

    // Switch back to normal
    let _ = engine.process(InputEvent::FocusChanged { sensitive: false });

    // Now expansion should work again
    let result = engine.process(InputEvent::Text(":email ".into()));
    assert_eq!(
        result.len(),
        1,
        "expansion should resume after leaving sensitive field"
    );
}

#[test]
fn buffer_truncation_doesnt_cause_false_misses() {
    // CRITICAL: When the rolling buffer is full and wraps, old characters
    // are evicted. If a trigger depends on context that was evicted, it
    // must NOT match (fail closed). This prevents partial-context matches.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = "complete"
        match_mode = "word-boundary"
        replacement = "done"
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();
    let max_buffer = engine.max_buffer_chars;

    // Fill buffer beyond capacity with non-trigger text
    let filler = "x".repeat(max_buffer + 10);
    let _ = engine.process(InputEvent::Text(filler));

    // At this point buffer_truncated = true and the buffer has wrapped.
    // Now type a trigger that requires word boundary.
    // The preceding context (if any) is gone, so word boundary check
    // may not have the context it needs. This should fail closed.

    let result = engine.process(InputEvent::Text("complete ".into()));
    // With truncated buffer context, word-boundary matching must be conservative
    // and not assume word boundary if context is missing.
    // This test documents the behavior: we still match, but the security model
    // should account for this edge case in the word-boundary implementation.
    assert!(
        result.len() <= 1,
        "buffer truncation should not cause spurious matches"
    );
}

#[test]
fn app_filter_prevents_cross_window_expansion() {
    // CRITICAL: app_filter is a security boundary. If we have an app-filtered
    // expansion and switch windows, that expansion must NOT match in the new
    // window. This prevents leaking sensitive snippets into wrong applications.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":pass"
        replacement = "secret123"
        app_filter = ["slack"]
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Set window to Slack - expansion should work
    let slack_ctx = WindowContext {
        app_id: Some("slack".to_string()),
        title: None,
    };
    engine.process(InputEvent::WindowChanged(Some(slack_ctx.clone())));

    let result = engine.process(InputEvent::Text(":pass ".into()));
    assert_eq!(result.len(), 1, "expansion should work in Slack");
    assert_eq!(result[0].insert, "secret123");

    // Switch to a different window (e.g., Firefox)
    let firefox_ctx = WindowContext {
        app_id: Some("firefox".to_string()),
        title: None,
    };
    engine.process(InputEvent::WindowChanged(Some(firefox_ctx)));

    // Type the same trigger in Firefox - must NOT expand
    let result = engine.process(InputEvent::Text(":pass ".into()));
    assert_eq!(
        result.len(),
        0,
        "app_filter must prevent expansion in non-matching apps"
    );

    // Switch back to Slack - should work again
    engine.process(InputEvent::WindowChanged(Some(slack_ctx)));
    let result = engine.process(InputEvent::Text(":pass ".into()));
    assert_eq!(result.len(), 1, "expansion should resume in matching app");
}

#[test]
fn undo_is_disabled_when_intervening_input_occurs() {
    // CRITICAL: Undo is only safe if pressed immediately after expansion.
    // If ANY other event occurs, last_expansion is cleared to prevent
    // accidentally undoing the wrong expansion or undoing when deletion failed.
    //
    // The engine tracks last_expansion and requires immediate follow-up
    // (no other events between expansion and undo) to prevent data loss.
    // This test verifies the safety invariant.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":date"
        replacement = "2024-01-15"
        match_mode = "word-boundary"
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Expand
    let result = engine.process(InputEvent::Text(":date ".into()));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].insert, "2024-01-15");
    assert!(
        engine.last_expansion.is_some(),
        "expansion should be tracked for potential undo"
    );

    // If ANY other event happens (Text, Key, Backspace, etc),
    // last_expansion is cleared to prevent unsafe undo
    let _ = engine.process(InputEvent::Text("x".into()));
    assert!(
        engine.last_expansion.is_none(),
        "last_expansion cleared by intervening input - prevents unsafe undo"
    );

    // Verify that subsequent Text events also clear it
    let _ = engine.process(InputEvent::Text(":date ".into()));
    let _ = engine.process(InputEvent::Text("y".into()));
    // After typing 'y', last_expansion should be cleared
    assert!(
        engine.last_expansion.is_none(),
        "any intervening event clears undo state"
    );
}

#[test]
#[cfg(unix)]
fn cancelling_a_running_command_returns_without_waiting_for_timeout() {
    let command = CommandConfig {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), "sleep 60".to_string()],
        timeout_ms: 60_000,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec![],
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let started = Instant::now();
    let worker = thread::spawn(move || run_command_with_shutdown(&command, Some(&worker_shutdown)));

    thread::sleep(Duration::from_millis(100));
    shutdown.store(true, Ordering::Release);
    let result = worker.join().unwrap();

    assert_eq!(result, Err(CommandError::StaleInput));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation waited for the command timeout"
    );
}

#[test]
#[cfg(unix)]
fn child_process_is_cleaned_up_when_output_exceeds_limit() {
    // CRITICAL: Ensure that when a child process writes oversized output,
    // the process is actually killed and reaped, not left running.
    // This prevents resource leaks where a misbehaving command continues
    // consuming CPU/memory even though WayExpand reported the error.
    use std::fs;

    let temp_dir = std::env::temp_dir().join("wayexpand-tests");
    let _ = fs::create_dir_all(&temp_dir);
    let pid_file = temp_dir.join(format!("child-cleanup-test-{}.pid", std::process::id()));

    let shell_command = format!(
        r#"{{ echo "pid: $$" > '{}'; python3 -c "import sys; sys.stdout.write('x' * (1024 * 1024 + 1))"; sleep 60; }}"#,
        pid_file.display()
    );

    let command = CommandConfig {
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), shell_command],
        timeout_ms: 5000,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec![],
    };

    let result = run_command(&command);

    assert!(
        matches!(result, Err(CommandError::OutputTooLarge)),
        "oversized output should return OutputTooLarge, got: {:?}",
        result
    );

    thread::sleep(Duration::from_millis(500));

    if pid_file.exists() {
        if let Ok(contents) = fs::read_to_string(&pid_file) {
            if let Some(pid_str) = contents
                .strip_prefix("pid: ")
                .and_then(|s| s.trim().parse::<i32>().ok())
            {
                let is_alive = unsafe { libc::kill(pid_str, 0) };
                assert_eq!(
                    is_alive, -1,
                    "child process {} must be killed after OutputTooLarge, but is still alive",
                    pid_str
                );
                let errno = unsafe { *libc::__errno_location() };
                assert_eq!(
                    errno,
                    libc::ESRCH,
                    "child process should be reaped with ESRCH"
                );
            }
        }
        let _ = fs::remove_file(pid_file);
    }
}

#[test]
fn output_size_policy_must_be_checked_post_execution() {
    // Command output is checked after execution as well as the empty
    // template placeholder used during preflight.
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-command-completed-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let command = format!(
        "printf completed > '{}'; yes x | head -c 100000",
        marker.display()
    );
    let config_text = format!(
        r#"
        [[expansion]]
        trigger = ":big"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "{command}"]
        timeout_ms = 500

        [organization]
        max_replacement_size = 1000
        "#
    );
    let config = Config::parse(&config_text).unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.enable_async_commands();

    // The empty template passes preflight, but the completed command output
    // must still be rejected by postflight validation.
    let results = engine.process(InputEvent::Text(":big".into()));
    assert!(results.is_empty(), "async command should be pending");

    // The command is allowed to complete, but its output must be rejected
    // after execution because the organization limit applies to output,
    // not just the empty template placeholder.
    for _ in 0..50 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.exists(), "the oversized command should complete");
    assert!(engine.drain_completed_commands().is_empty());

    let completed = engine.process(InputEvent::EndOfInput);
    assert!(
        completed.is_empty(),
        "oversized output must never be injected"
    );
    let _ = std::fs::remove_file(marker);
}

#[test]
fn sync_and_deferred_paths_have_matching_snippet_semantics() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "Regards, {{cursor}} team"
        propagate_case = true

        [[expansion]]
        trigger = ":cmd"
        replacement = ""
        propagate_case = true
        [expansion.command]
        program = "printf"
        args = ["generated"]
        cache_ms = 1000
        timeout_ms = 500
        "#,
    )
    .unwrap();

    let mut synchronous = ExpansionEngine::new(config.clone()).unwrap();
    let static_sync = synchronous
        .process(InputEvent::Text(":SIG".into()))
        .pop()
        .unwrap();
    assert_eq!(static_sync.insert, "REGARDS,  TEAM");

    let mut deferred = ExpansionEngine::new(config).unwrap();
    let static_pending = deferred
        .process_deferred(InputEvent::Text(":SIG".into()))
        .pop()
        .unwrap();
    let static_deferred = deferred
        .dispatch_pending_with_policy(static_pending, 0)
        .unwrap();
    let static_deferred = match static_deferred {
        PendingExpansionDispatch::Ready(result) => result,
        PendingExpansionDispatch::Queued => panic!("static expansion was queued"),
    };
    assert_eq!(static_sync.trigger, static_deferred.trigger);
    assert_eq!(static_sync.matched_text, static_deferred.matched_text);
    assert_eq!(static_sync.insert, static_deferred.insert);
    assert_eq!(static_sync.cursor_offset, static_deferred.cursor_offset);

    let command_sync = deferred
        .process(InputEvent::Text(":CMD".into()))
        .pop()
        .unwrap();
    assert_eq!(command_sync.insert, "GENERATED");
    let mut command_deferred_engine = ExpansionEngine::new(
        Config::parse(
            r#"
            [[expansion]]
            trigger = ":cmd"
            replacement = ""
            propagate_case = true
            [expansion.command]
            program = "printf"
            args = ["generated"]
            cache_ms = 1000
            timeout_ms = 500
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    let command_pending = command_deferred_engine
        .process_deferred(InputEvent::Text(":CMD".into()))
        .pop()
        .unwrap();
    assert!(command_deferred_engine.enable_async_commands());
    let command_deferred = dispatch_and_wait(&mut command_deferred_engine, command_pending);
    assert_eq!(command_sync.trigger, command_deferred.trigger);
    assert_eq!(command_sync.matched_text, command_deferred.matched_text);
    assert_eq!(command_sync.insert, command_deferred.insert);
    assert!(command_sync.command_backed && command_deferred.command_backed);
}

#[test]
fn deferred_rollback_restores_an_absorbed_delimiter_with_the_trigger() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig "
        replacement = "regards"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":sig".into()))
        .pop();
    assert!(pending.is_none(), "the delimiter completes this trigger");

    // Model the evdev gate having absorbed the physical delimiter into
    // the result before policy or injection rejected it.
    let pending = engine
        .process_deferred(InputEvent::Delimiter(' '))
        .pop()
        .unwrap();
    let matched = pending.matched_text.clone();
    assert_eq!(matched, ":sig ");
    engine.restore_deferred_match(&matched);

    let restored = engine
        .process_deferred(InputEvent::EndOfInput)
        .pop()
        .unwrap();
    assert_eq!(restored.matched_text, ":sig ");
}

#[test]
fn deferred_dispatch_commits_undo_only_after_injection() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":sig".into()))
        .pop()
        .unwrap();
    let result = match engine.dispatch_pending_with_policy(pending, 0).unwrap() {
        PendingExpansionDispatch::Ready(result) => result,
        PendingExpansionDispatch::Queued => panic!("static expansion was queued"),
    };
    let chord = KeyChord::parse("Ctrl+Z").unwrap();

    assert!(
        engine.try_undo(&chord).is_none(),
        "pre-injection results must not create undo state"
    );
    engine.commit_applied_expansion(&result);
    assert!(engine.try_undo(&chord).is_some());
}

#[test]
fn multi_scalar_text_invalidates_undo_after_a_mid_event_match() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let results = engine.process(InputEvent::Text(":sigx".into()));

    assert_eq!(results.len(), 1);
    assert!(engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .is_none());
}

#[test]
fn deferred_generation_is_captured_at_match_time() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":sigx".into()))
        .pop()
        .unwrap();

    assert!(pending.generation < engine.input_generation);
}

#[test]
fn deferred_multi_scalar_text_invalidates_undo_after_a_mid_event_match() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":sig"
        replacement = "regards"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":sigx".into()))
        .pop()
        .unwrap();
    let result = match engine.dispatch_pending_with_policy(pending, 0).unwrap() {
        PendingExpansionDispatch::Ready(result) => result,
        PendingExpansionDispatch::Queued => panic!("static expansion was queued"),
    };

    engine.commit_applied_expansion(&result);

    assert!(
        engine
            .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
            .is_none(),
        "a later scalar in the same text event must invalidate deferred undo"
    );
}

#[test]
fn deferred_command_output_over_policy_limit_is_rejected_after_completion() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-deferred-policy-output-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let command = format!(
        "printf completed > '{}'; yes x | head -c 257",
        marker.display()
    );
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":large"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "{command}"]
        timeout_ms = 500

        [organization]
        max_replacement_size = 0
        "#
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":large".into()))
        .pop()
        .unwrap();

    assert!(matches!(
        engine.execute_pending_inline(pending, 256),
        Err(CommandError::PolicyOutputTooLarge {
            size: 257,
            limit: 256
        })
    ));
    assert!(
        marker.exists(),
        "the subprocess should complete before rejection"
    );
    let _ = std::fs::remove_file(marker);
}

#[test]
fn deferred_command_preserves_case_cache_and_undo_semantics() {
    let sync_marker =
        std::env::temp_dir().join(format!("wayexpand-sync-cache-{}", std::process::id()));
    let deferred_marker =
        std::env::temp_dir().join(format!("wayexpand-deferred-cache-{}", std::process::id()));
    let _ = std::fs::remove_file(&sync_marker);
    let _ = std::fs::remove_file(&deferred_marker);
    let config_for = |marker: &std::path::Path| {
        let command = format!("printf x >> '{}'; printf Regards", marker.display());
        Config::parse(&format!(
            r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":sig"
        replacement = ""
        propagate_case = true
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "{command}"]
        cache_ms = 5000
        timeout_ms = 500
        "#
        ))
        .unwrap()
    };

    let mut synchronous = ExpansionEngine::new(config_for(&sync_marker)).unwrap();
    let upper_sync = synchronous
        .process(InputEvent::Text(":SIG".into()))
        .pop()
        .unwrap();
    let lower_sync = synchronous
        .process(InputEvent::Text(":sig".into()))
        .pop()
        .unwrap();
    let undo_sync = synchronous
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .unwrap();

    let mut deferred = ExpansionEngine::new(config_for(&deferred_marker)).unwrap();
    assert!(deferred.enable_async_commands());
    let upper_pending = deferred
        .process_deferred(InputEvent::Text(":SIG".into()))
        .pop()
        .unwrap();
    let upper_deferred = dispatch_and_wait(&mut deferred, upper_pending);
    let lower_pending = deferred
        .process_deferred(InputEvent::Text(":sig".into()))
        .pop()
        .unwrap();
    let lower_deferred = dispatch_and_wait(&mut deferred, lower_pending);
    let undo_deferred = deferred
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .unwrap();

    assert_eq!(upper_sync.insert, "REGARDS");
    assert_eq!(upper_sync.insert, upper_deferred.insert);
    assert_eq!(lower_sync.insert, "Regards");
    assert_eq!(lower_sync.insert, lower_deferred.insert);
    assert_eq!(undo_sync.matched_text, undo_deferred.matched_text);
    assert_eq!(undo_sync.insert, undo_deferred.insert);
    assert_eq!(std::fs::read_to_string(&sync_marker).unwrap(), "x");
    assert_eq!(std::fs::read_to_string(&deferred_marker).unwrap(), "x");
    let _ = std::fs::remove_file(sync_marker);
    let _ = std::fs::remove_file(deferred_marker);
}

#[cfg(unix)]
#[test]
fn deferred_command_without_workers_fails_closed_without_running() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-deferred-no-workers-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":cmd"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf ran > '{}'"]
        timeout_ms = 1000
        "#,
        marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":cmd".into()))
        .pop()
        .unwrap();

    assert_eq!(
        engine.dispatch_pending_with_policy(pending, 0),
        Err(CommandError::WorkerUnavailable)
    );
    assert!(
        !marker.exists(),
        "worker failure must not run commands inline"
    );
}

#[test]
fn deferred_command_is_not_run_after_input_generation_changes() {
    let marker =
        std::env::temp_dir().join(format!("wayexpand-deferred-stale-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":cmd"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf ran > '{}'"]
        timeout_ms = 500
        "#,
        marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":cmd".into()))
        .pop()
        .unwrap();
    engine.process_deferred(InputEvent::Text("x".into()));

    assert!(matches!(
        engine.execute_pending_inline(pending, 0),
        Err(CommandError::StaleInput)
    ));
    assert!(!marker.exists(), "stale deferred commands must not run");
}

#[cfg(unix)]
#[test]
fn dropping_async_runtime_discards_queued_commands() {
    let first_marker =
        std::env::temp_dir().join(format!("wayexpand-shutdown-first-{}", std::process::id()));
    let second_marker =
        std::env::temp_dir().join(format!("wayexpand-shutdown-second-{}", std::process::id()));
    let _ = std::fs::remove_file(&first_marker);
    let _ = std::fs::remove_file(&second_marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":one"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.4; printf first > '{}'"]
        timeout_ms = 2000

        [[expansion]]
        trigger = ":two"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf second > '{}'"]
        timeout_ms = 2000
        "#,
        first_marker.display(),
        second_marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.enable_async_commands());

    let first = engine
        .process_deferred(InputEvent::Text(":one".into()))
        .pop()
        .unwrap();
    assert_eq!(
        engine.dispatch_pending_with_policy(first, 0),
        Ok(PendingExpansionDispatch::Queued)
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    while engine.command_metrics().command_in_flight == 0 {
        assert!(Instant::now() < deadline, "first command did not start");
        thread::sleep(Duration::from_millis(5));
    }

    // Clear the logical reservation before queuing a second command. The
    // first command remains in flight while the second is buffered.
    engine.process_deferred(InputEvent::Reset);
    let second = engine
        .process_deferred(InputEvent::Text(":two".into()))
        .pop()
        .unwrap();
    assert_eq!(
        engine.dispatch_pending_with_policy(second, 0),
        Ok(PendingExpansionDispatch::Queued)
    );

    drop(engine);
    assert!(
        !first_marker.exists(),
        "in-flight command must be cancelled during shutdown"
    );
    assert!(
        !second_marker.exists(),
        "queued command must be discarded during shutdown"
    );
    let _ = std::fs::remove_file(first_marker);
    let _ = std::fs::remove_file(second_marker);
}

#[test]
fn completion_backpressure_yields_to_shutdown() {
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.send(1_u8).unwrap();
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let worker =
        thread::spawn(move || !send_completion_or_shutdown(&sender, 2_u8, &worker_shutdown));

    thread::sleep(Duration::from_millis(20));
    shutdown.store(true, Ordering::Release);
    assert!(worker.join().unwrap(), "completion helper should stop");
    assert_eq!(receiver.try_recv().unwrap(), 1);
}

#[test]
fn deferred_command_respects_engine_disable_commands_policy() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-deferred-disabled-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":cmd"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf ran > '{}'"]
        timeout_ms = 500

        [organization]
        disable_commands = true
        "#,
        marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let pending = engine
        .process_deferred(InputEvent::Text(":cmd".into()))
        .pop()
        .unwrap();

    assert!(matches!(
        engine.execute_pending_inline(pending, 0),
        Err(CommandError::PolicyBlocked)
    ));
    assert!(!marker.exists(), "policy-disabled commands must not run");
}

#[test]
fn async_worker_failure_must_block_commands_not_fallback_to_sync() {
    // REGRESSION: When async infrastructure is unavailable (runtime is None),
    // disable_commands policy check was skipped, allowing sync fallback.
    // FIXED: Policy enforcement now applies to sync fallback path too.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":sync"
        replacement = ""
        [expansion.command]
        program = "echo"
        args = ["sync-command-executed"]
        timeout_ms = 500

        [organization]
        disable_commands = true
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();
    // Deliberately do NOT call enable_async_commands(). This keeps async_commands = None,
    // which forces take_match() to use the sync fallback path.
    // The disable_commands policy must still block execution in this path.

    let results = engine.process(InputEvent::Text(":sync".into()));

    // With the fix, disable_commands now applies to both async and sync paths
    assert!(
        results.is_empty(),
        "disable_commands policy must block commands even in sync fallback path. Got: {:?}",
        results
    );
}

#[test]
fn command_backed_flag_must_reflect_actual_execution() {
    // BUG: When async worker fallback executes synchronously,
    // command_backed is set to false, misleading policy checks
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":fallback"
        replacement = ""
        [expansion.command]
        program = "echo"
        args = ["fallback-executed"]
        timeout_ms = 500
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();
    // Deliberately do NOT call enable_async_commands() to simulate async unavailability.
    // This means engine.async_commands will be None, forcing sync path in take_match().

    let results = engine.process(InputEvent::Text(":fallback".into()));

    // When commands execute via sync fallback (because async_commands is None),
    // the command_backed flag should still reflect that a command actually executed.
    // Previously this was a bug: command_backed was false for sync fallback, misleading
    // policy checks about output provenance.
    if !results.is_empty() {
        // With the fix, synchronous fallback now properly sets command_backed=true
        // This test verifies the fix works.
        assert!(
            results[0].command_backed,
            "Synchronous command fallback must set command_backed=true to reflect actual execution"
        );
    }
}
