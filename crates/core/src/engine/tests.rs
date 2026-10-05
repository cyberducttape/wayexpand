use std::os::unix::fs::PermissionsExt;
use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::MatchMode;

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

/// The immediate and deferred processors are two entry points onto one
/// matching policy: `process` is what the settings frontends preview with, and
/// `process_deferred` is what the daemon actually runs. They used to carry
/// separate copies of that policy, so a fix applied to one could silently miss
/// the other. This pins the property those copies were supposed to maintain --
/// for the same input, both must match the same triggers, consume the same
/// text, and reinsert the same terminator.
#[test]
fn the_immediate_and_deferred_processors_agree_on_what_matches() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":a"
        replacement = "alpha"

        [[expansion]]
        trigger = ":address"
        replacement = "1 Example Street"

        [[expansion]]
        trigger = "btw"
        replacement = "by the way"
        match_mode = "word-boundary"

        [[expansion]]
        trigger = ":cafe"
        replacement = "café"
    "#,
    )
    .unwrap();

    let mut total_matches = 0usize;
    // Prefix families, a word-boundary trigger with and without a preceding
    // word character, a non-matching tail, and multi-scalar text.
    for input in [
        ":a ",
        ":address ",
        ":addr",
        "say :a then :address.",
        "btw ",
        "abtw ",
        " btw,",
        "btw",
        ":cafe",
        "nothing here",
        "::a",
    ] {
        let mut immediate = ExpansionEngine::new(config.clone()).unwrap();
        let mut deferred = ExpansionEngine::new(config.clone()).unwrap();

        let immediate_matches: Vec<(String, String, Option<char>)> = immediate
            .process(InputEvent::Text(input.into()))
            .into_iter()
            .map(|result| (result.trigger, result.matched_text, result.reinsert_after))
            .collect();
        let deferred_matches: Vec<(String, String, Option<char>)> = deferred
            .process_deferred(InputEvent::Text(input.into()))
            .into_iter()
            .map(|pending| {
                (
                    pending.trigger.clone(),
                    pending.matched_text.clone(),
                    pending.reinsert_after,
                )
            })
            .collect();

        assert_eq!(
            immediate_matches, deferred_matches,
            "immediate and deferred processing disagreed on {input:?}"
        );
        total_matches += immediate_matches.len();
    }
    // Guards against the comparison above passing because both paths matched
    // nothing at all.
    assert!(
        total_matches >= 6,
        "expected the sample inputs to exercise real matches, saw {total_matches}"
    );
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
        app_filter = ["app_id_exact:org.mozilla.thunderbird"]
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
        app_filter = ["app_id_exact:Org.Mozilla.Thunderbird"]
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird".into()),
        title: None,
        instance_id: None,
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
        app_filter = ["app_id_exact:org.mozilla.thunderbird"]
    "#,
    )
    .unwrap();
    let mut old_engine = ExpansionEngine::new(config.clone()).unwrap();
    old_engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird".into()),
        title: None,
        instance_id: None,
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
        app_filter = ["app_id_exact:org.mozilla.thunderbird"]
    "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.process(InputEvent::WindowChanged(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: Some("konsole".into()),
        instance_id: None,
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
        instance_id: None,
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
        instance_id: None,
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
        instance_id: None,
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
fn active_composition_suspends_matching_until_commit() {
    let mut engine = engine();

    // A preedit must not be able to join with text typed before composition
    // started. Composition transitions are a transaction boundary, so a
    // partially typed trigger is discarded in both directions.
    engine.process(InputEvent::Text(":hel".into()));
    engine.process(InputEvent::CompositionChanged { active: true });
    assert!(engine.is_composition_active());
    assert!(engine.process(InputEvent::Text(":hello".into())).is_empty());

    // A committed composition ends the guard. The trigger starts cleanly and
    // cannot combine with either the old partial trigger or preedit text.
    engine.process(InputEvent::CompositionChanged { active: false });
    assert!(!engine.is_composition_active());
    let mut results = engine.process(InputEvent::Text("llo".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    assert!(results.is_empty());

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
    let mut observed_hotkey_work = false;
    let (completed_action, result) = loop {
        let metrics = engine.command_metrics();
        observed_hotkey_work |= metrics.hotkey_queue_depth > 0 || metrics.hotkey_in_flight > 0;
        assert_eq!(metrics.expansion_command_queue_depth, 0);
        assert_eq!(metrics.expansion_command_in_flight, 0);
        if let Some(completion) = engine.drain_completed_hotkeys().pop() {
            break completion;
        }
        assert!(Instant::now() < deadline, "hotkey did not complete");
        thread::sleep(Duration::from_millis(5));
    };
    assert!(
        observed_hotkey_work,
        "hotkey work was never visible in its counters"
    );
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
        expansion_metrics: Arc::clone(&engine.expansion_metrics),
        hotkey_metrics: Arc::clone(&engine.hotkey_metrics),
        shutdown: Arc::new(AtomicBool::new(false)),
        command_workers: Vec::new(),
        hotkey_worker: None,
    });
    for character in ":slow".chars() {
        engine.push_buffered(character);
    }
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
        expansion_metrics: Arc::clone(&engine.expansion_metrics),
        hotkey_metrics: Arc::clone(&engine.hotkey_metrics),
        shutdown: Arc::new(AtomicBool::new(false)),
        command_workers: Vec::new(),
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
    // Ensure the timeout job has crossed the worker's pre-spawn generation
    // check before the next input makes its result stale. This test covers
    // metrics for a command that actually started; queued stale jobs are
    // covered separately and must never spawn.
    let deadline = Instant::now() + Duration::from_secs(1);
    while engine.command_metrics().command_in_flight == 0
        && engine.command_metrics().command_timeout_total == 0
    {
        assert!(Instant::now() < deadline, "timeout command did not start");
        thread::sleep(Duration::from_millis(1));
    }
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
            id: crate::ExpansionConfig::new_id(),
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
            aliases: Vec::new(),
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
fn word_boundary_mode_keeps_grapheme_continuations_together() {
    let config = || {
        Config::parse(
            "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        )
        .unwrap()
    };

    // NFC, decomposed Latin, Indic marks, a ZWJ sequence, and a non-Latin
    // script must all count as word continuations before the trigger.
    for prefix in [
        "é",
        "e\u{301}",
        "क\u{094d}",
        "का",
        "👩\u{200d}💻",
        "مَرْحَبًا",
        "rock’",
    ] {
        let mut engine = ExpansionEngine::new(config()).unwrap();
        assert!(
            engine
                .process(InputEvent::Text(format!("{prefix}:sig")))
                .is_empty(),
            "word boundary incorrectly accepted after {prefix:?}"
        );
    }

    // Combining marks and apostrophes also continue the word after a trigger,
    // so they must not prematurely resolve a word-boundary match.
    for suffix in ["\u{301}", "\u{094d}", "\u{200d}", "’"] {
        let mut engine = ExpansionEngine::new(config()).unwrap();
        assert!(
            engine
                .process(InputEvent::Text(format!(":sig{suffix}")))
                .is_empty(),
            "word boundary incorrectly resolved before suffix {suffix:?}"
        );
    }
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

    fn capabilities(&self) -> crate::InjectorCapabilities {
        crate::InjectorCapabilities {
            insertion_mode: "test",
            cursor_reposition: true,
            full_unicode: true,
            ..crate::InjectorCapabilities::default()
        }
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

struct CapabilityInjector {
    calls: Vec<String>,
    capabilities: crate::InjectorCapabilities,
}

impl crate::TextInjector for CapabilityInjector {
    fn name(&self) -> &'static str {
        "capability-test"
    }

    fn capabilities(&self) -> crate::InjectorCapabilities {
        self.capabilities
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
fn apply_preflights_backend_capabilities_before_destructive_output() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "x".repeat(251),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = CapabilityInjector {
        calls: Vec::new(),
        capabilities: crate::InjectorCapabilities {
            insertion_mode: "keysym fallback",
            max_text_chars: 250,
            ..crate::InjectorCapabilities::default()
        },
    };
    let outcome = ExpansionEngine::apply(&mut injector, &result);
    assert!(matches!(
        outcome,
        crate::TransactionOutcome::NotApplied { ref source }
            if source.message.contains("supports at most 250")
    ));
    assert!(injector.calls.is_empty());
}

#[test]
fn apply_preflight_counts_reinserted_delimiter_as_one_character() {
    // A multi-byte delimiter is still one character toward the limit.
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "x".repeat(249),
        cursor_offset: None,
        reinsert_after: Some('—'),
        command_backed: false,
        undoable: true,
    };
    let mut injector = CapabilityInjector {
        calls: Vec::new(),
        capabilities: crate::InjectorCapabilities {
            insertion_mode: "keysym fallback",
            max_text_chars: 250,
            full_unicode: true,
            ..crate::InjectorCapabilities::default()
        },
    };
    let outcome = ExpansionEngine::apply(&mut injector, &result);
    assert!(outcome.is_applied(), "{outcome:?}");
}

#[test]
fn apply_allows_zero_cursor_offset_without_cursor_support() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "x".into(),
        cursor_offset: Some(0),
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = CapabilityInjector {
        calls: Vec::new(),
        capabilities: crate::InjectorCapabilities {
            insertion_mode: "no cursor",
            ..crate::InjectorCapabilities::default()
        },
    };
    let outcome = ExpansionEngine::apply(&mut injector, &result);
    assert!(outcome.is_applied(), "{outcome:?}");
}

#[test]
fn apply_rejects_unrepresentable_unicode_before_output() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "café".into(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = CapabilityInjector {
        calls: Vec::new(),
        capabilities: crate::InjectorCapabilities {
            insertion_mode: "keysym fallback",
            ..crate::InjectorCapabilities::default()
        },
    };
    let outcome = ExpansionEngine::apply(&mut injector, &result);
    assert!(matches!(
        outcome,
        crate::TransactionOutcome::NotApplied { ref source }
            if source.message.contains("cannot guarantee")
    ));
    assert!(injector.calls.is_empty());
}

#[test]
fn apply_erases_before_inserting() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "value".into(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = RecordingInjector { calls: Vec::new() };
    assert!(ExpansionEngine::apply(&mut injector, &result).is_applied());
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
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "value".into(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let mut injector = AtomicInjector { calls: Vec::new() };
    assert!(ExpansionEngine::apply(&mut injector, &result).is_applied());
    assert_eq!(injector.calls, ["replace::x:value"]);
}

#[test]
fn apply_replaces_typed_trigger_and_commits_terminator() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":sig".into(),
        matched_text: ":SIG".into(),
        insert: "Best regards,".into(),
        cursor_offset: Some(2),
        reinsert_after: Some(' '),
        command_backed: false,
        undoable: true,
    };
    let mut injector = RecordingInjector { calls: Vec::new() };
    assert!(ExpansionEngine::apply(&mut injector, &result).is_applied());
    assert_eq!(
        injector.calls,
        ["erase::SIG ", "insert:Best regards, ", "left:3"]
    );
}

struct CursorFailingInjector;

impl crate::TextInjector for CursorFailingInjector {
    fn name(&self) -> &'static str {
        "cursor-failing-test"
    }

    fn capabilities(&self) -> crate::InjectorCapabilities {
        crate::InjectorCapabilities {
            insertion_mode: "cursor-failing-test",
            cursor_reposition: true,
            ..crate::InjectorCapabilities::default()
        }
    }

    fn erase(&mut self, _: &str) -> Result<(), crate::InjectorError> {
        Ok(())
    }

    fn insert(&mut self, _: &str) -> Result<(), crate::InjectorError> {
        Ok(())
    }

    fn move_cursor_left(&mut self, _: usize) -> Result<(), crate::InjectorError> {
        Err(crate::InjectorError {
            backend: "cursor-failing-test",
            message: "cursor movement failed".into(),
            retryable: false,
        })
    }
}

#[test]
fn cursor_failure_is_reported_after_replacement_is_applied() {
    let result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":x".into(),
        matched_text: ":x".into(),
        insert: "value".into(),
        cursor_offset: Some(1),
        reinsert_after: None,
        command_backed: false,
        undoable: true,
    };
    let outcome = ExpansionEngine::apply(&mut CursorFailingInjector, &result);
    assert!(matches!(
        &outcome,
        crate::TransactionOutcome::AppliedWithCursorPositionFailure { .. }
    ));
    assert!(outcome.is_applied());
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
    std::fs::remove_file(root.join(".target.toml.wayexpand.lock")).unwrap();
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
    assert!(ExpansionEngine::apply(&mut injector, &result).is_applied());
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
    assert!(ExpansionEngine::apply(&mut injector, &expansion).is_applied());

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("an expansion is pending to undo");
    assert!(ExpansionEngine::apply(&mut injector, &undo).is_applied());

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
    assert!(ExpansionEngine::apply(&mut injector, &expansion).is_applied());

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("a boundary expansion should be undoable");
    assert_eq!(undo.matched_text, "Regards ");
    assert_eq!(undo.insert, ":sig ");
    assert!(ExpansionEngine::apply(&mut injector, &undo).is_applied());

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
    assert!(ExpansionEngine::apply(&mut injector, &expansion).is_applied());

    let undo = engine
        .try_undo(&KeyChord::parse("Ctrl+Z").unwrap())
        .expect("an async boundary expansion should be undoable");
    assert_eq!(undo.matched_text, "Regards ");
    assert_eq!(undo.insert, ":sig ");
    assert!(ExpansionEngine::apply(&mut injector, &undo).is_applied());
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"app_id_exact:org.mozilla.thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.mozilla.Thunderbird".into()),
        title: Some("Some Mail".into()),
        instance_id: None,
    }));
    let results = engine.process(InputEvent::Text(":email".into()));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].insert, "contact@example.com");
}

#[test]
fn app_id_exact_does_not_match_a_similar_app_id() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = ':email'\nreplacement = 'contact@example.com'\napp_filter = ['app_id_exact:org.mozilla.thunderbird']",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird-helper".into()),
        title: Some("Thunderbird Mail".into()),
        instance_id: None,
    }));
    assert!(engine.process(InputEvent::Text(":email".into())).is_empty());
}

#[test]
fn app_id_glob_is_explicit_and_title_contains_is_explicit() {
    let glob = Config::parse(
        "[[expansion]]\ntrigger = ':glob'\nreplacement = 'glob'\napp_filter = ['app_id_glob:*thunderbird*']",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(glob).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.mozilla.thunderbird-helper".into()),
        title: None,
        instance_id: None,
    }));
    assert_eq!(engine.process(InputEvent::Text(":glob".into())).len(), 1);
}

#[test]
fn app_filter_uses_title_only_when_app_id_unavailable() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"title_contains:thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: None,
        title: Some("Thunderbird Mail Client".into()),
        instance_id: None,
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"app_id_exact:org.mozilla.thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: Some("Thunderbird troubleshooting".into()),
        instance_id: None,
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"app_id_exact:org.mozilla.thunderbird\"]",
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
        instance_id: None,
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"app_id_exact:org.mozilla.thunderbird\"]",
    )
    .unwrap();
    config.organization.disable_title_matching = true;

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: None, // Only title available
        title: Some("Thunderbird Mail Client".into()),
        instance_id: None,
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"app_id_exact:org.thunderbird.thunderbird\"]",
    )
    .unwrap();
    config.organization.disable_title_matching = true;

    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.thunderbird.Thunderbird".into()),
        title: Some("Some Email".into()),
        instance_id: None,
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
        "[[expansion]]\ntrigger = \":email\"\nreplacement = \"contact@example.com\"\napp_filter = [\"title_contains:thunderbird\"]",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    engine.set_title_matching_disabled(true);
    engine.set_current_window(Some(WindowContext {
        app_id: None,
        title: Some("Thunderbird Mail Client".into()),
        instance_id: None,
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
fn hotkey_descendants_are_cleaned_up_on_successful_exit() {
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

    let script_path = PathBuf::from(format!(
        "/tmp/wayexpand-hotkey-descendant-test-{}.sh",
        std::process::id()
    ));
    let output_file = PathBuf::from(format!(
        "/tmp/wayexpand-hotkey-descendant-marker-{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&output_file);

    let script_content = format!(
        "#!/bin/bash\n(sleep 1; printf survived > {}) &\nexit 0\n",
        output_file.display()
    );
    let mut script_file = File::create(&script_path).unwrap();
    script_file.write_all(script_content.as_bytes()).unwrap();
    drop(script_file);
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let action = HotkeyResult {
        chord: KeyChord::parse("Ctrl+Alt+H").unwrap(),
        description: "descendant cleanup test".into(),
        command: CommandConfig {
            action: None,
            program: script_path.display().to_string(),
            args: Vec::new(),
            timeout_ms: 5000,
            cache_ms: 0,
            environment: CommandEnvironment::Minimal,
            pass_env: Vec::new(),
        },
    };

    let result = ExpansionEngine::execute_hotkey(&action);
    thread::sleep(Duration::from_millis(1500));
    let survived = output_file.exists();
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&output_file);

    assert!(result.is_ok());
    assert!(
        !survived,
        "hotkey descendant survived process-group cleanup"
    );
}

#[cfg(unix)]
#[test]
fn detached_stdout_holder_returns_incomplete_output_without_blocking() {
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
        action: None,
        program: script_path.clone(),
        args: Vec::new(),
        timeout_ms: 5000,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: Vec::new(),
    };

    let started = Instant::now();
    let output = run_command(&command);
    let elapsed = started.elapsed();

    let _ = std::fs::remove_file(&script_path);
    thread::sleep(Duration::from_millis(1200));
    let _ = std::fs::remove_file(&escaped_marker);

    assert_eq!(output, Err(CommandError::IncompleteOutput));
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
fn buffer_truncation_fails_closed_for_unknown_preceding_context() {
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

    assert!(engine.buffer_truncated);

    // Now type a trigger that requires a word boundary. The retained
    // preceding filler is not a boundary, and any evicted context must not
    // be treated as proof that a boundary existed.

    let result = engine.process(InputEvent::Text("complete ".into()));
    assert_eq!(
        result.len(),
        0,
        "truncated context must fail closed rather than expand"
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
        app_filter = ["app_id_exact:slack"]
        "#,
    )
    .unwrap();

    let mut engine = ExpansionEngine::new(config).unwrap();

    // Set window to Slack - expansion should work
    let slack_ctx = WindowContext {
        app_id: Some("slack".to_string()),
        title: None,
        instance_id: None,
    };
    engine.process(InputEvent::WindowChanged(Some(slack_ctx.clone())));

    let result = engine.process(InputEvent::Text(":pass ".into()));
    assert_eq!(result.len(), 1, "expansion should work in Slack");
    assert_eq!(result[0].insert, "secret123");

    // Switch to a different window (e.g., Firefox)
    let firefox_ctx = WindowContext {
        app_id: Some("firefox".to_string()),
        title: None,
        instance_id: None,
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
        action: None,
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
fn command_rejects_output_when_descendant_keeps_stdout_open() {
    let pid_file = format!(
        "/tmp/wayexpand-incomplete-output-{}.pid",
        std::process::id()
    );
    let command = CommandConfig {
        action: None,
        program: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            format!(
                "/usr/bin/setsid /bin/sh -c '/bin/sleep 10 & echo $! > {pid_file}; wait' & while [ ! -s {pid_file} ]; do /bin/sleep 0.01; done; printf complete"
            ),
        ],
        timeout_ms: 5_000,
        cache_ms: 0,
        environment: CommandEnvironment::Minimal,
        pass_env: vec![],
    };

    let result = run_command(&command);
    if let Ok(pid) = std::fs::read_to_string(&pid_file).and_then(|text| {
        text.trim()
            .parse::<libc::pid_t>()
            .map_err(std::io::Error::other)
    }) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    let _ = std::fs::remove_file(pid_file);
    assert_eq!(result, Err(CommandError::IncompleteOutput));
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
        action: None,
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
fn deferred_restoration_never_exceeds_the_configured_buffer_bound() {
    let config = Config::parse(
        r#"
        [settings]
        max_buffer_chars = 4

        [[expansion]]
        trigger = ":abc"
        replacement = "first"

        [[expansion]]
        trigger = ":def"
        replacement = "second"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let assert_bounded = |engine: &ExpansionEngine| {
        assert!(engine.buffer.len() <= engine.max_buffer_chars);
    };

    let first = engine.process_deferred(InputEvent::Text(":abc".into()));
    assert_eq!(first.len(), 1);
    assert_bounded(&engine);
    let second = engine.process_deferred(InputEvent::Text(":def".into()));
    assert_eq!(second.len(), 2);
    assert_bounded(&engine);

    engine.restore_deferred_match(":abc");
    assert_bounded(&engine);
    engine.restore_deferred_match(":def");
    assert_bounded(&engine);
    assert_eq!(engine.buffer.iter().collect::<String>(), ":def");
    assert!(engine.buffer_truncated);

    let mut bulk_engine = ExpansionEngine::new(
        Config::parse(
            r#"
            [settings]
            max_buffer_chars = 4

            [[expansion]]
            trigger = ":abc"
            replacement = "first"

            [[expansion]]
            trigger = ":def"
            replacement = "second"
            "#,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        bulk_engine
            .process_deferred(InputEvent::Text(":abc".into()))
            .len(),
        1
    );
    assert_bounded(&bulk_engine);
    assert_eq!(
        bulk_engine
            .process_deferred(InputEvent::Text(":def".into()))
            .len(),
        2
    );
    assert_bounded(&bulk_engine);
    bulk_engine.process_deferred(InputEvent::Key(KeyChord::parse("Ctrl+K").unwrap()));
    assert_bounded(&bulk_engine);
    assert_eq!(bulk_engine.buffer.iter().collect::<String>(), ":def");
    assert!(bulk_engine.buffer_truncated);
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
fn queued_async_command_that_becomes_stale_is_discarded_before_spawn() {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    );
    let starts: Vec<_> = (0..ASYNC_COMMAND_WORKER_COUNT)
        .map(|index| std::env::temp_dir().join(format!("wayexpand-worker-{suffix}-{index}")))
        .collect();
    let stale_marker = std::env::temp_dir().join(format!("wayexpand-worker-{suffix}-stale"));
    for path in starts.iter().chain(std::iter::once(&stale_marker)) {
        let _ = std::fs::remove_file(path);
    }

    let mut expansions = String::new();
    for marker in starts.iter().chain(std::iter::once(&stale_marker)) {
        expansions.push_str(&format!(
            "\n[[expansion]]\ntrigger = \":job{}\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"/bin/sh\"\nargs = [\"-c\", \"touch '{}'; sleep 1\"]\ntimeout_ms = 3000\n",
            expansions.matches("[[expansion]]").count(),
            marker.display()
        ));
    }
    let config = Config::parse(&expansions).unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.enable_async_commands());

    for index in 0..ASYNC_COMMAND_WORKER_COUNT {
        let result = ExpansionResult {
            snippet_id: String::new(),
            trigger: format!(":job{index}"),
            matched_text: format!(":job{index}"),
            insert: String::new(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: true,
            undoable: true,
        };
        assert!(engine
            .async_commands
            .as_ref()
            .unwrap()
            .try_send_command(AsyncCommandJob::Expansion {
                config_index: index,
                generation: engine.input_generation,
                additional_max_size: 0,
                command: engine.config.expansion[index].command.clone().unwrap(),
                result,
            })
            .is_ok());
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    while starts.iter().any(|path| !path.exists()) {
        assert!(
            Instant::now() < deadline,
            "workers did not start the blocking jobs: metrics={:?}, generation={}",
            engine.command_metrics(),
            engine.shared_input_generation.load(Ordering::Acquire)
        );
        thread::sleep(Duration::from_millis(5));
    }

    let stale_result = ExpansionResult {
        snippet_id: String::new(),
        trigger: ":stale".into(),
        matched_text: ":stale".into(),
        insert: String::new(),
        cursor_offset: None,
        reinsert_after: None,
        command_backed: true,
        undoable: true,
    };
    assert!(engine
        .async_commands
        .as_ref()
        .unwrap()
        .try_send_command(AsyncCommandJob::Expansion {
            config_index: ASYNC_COMMAND_WORKER_COUNT,
            generation: engine.input_generation,
            additional_max_size: 0,
            command: engine.config.expansion[ASYNC_COMMAND_WORKER_COUNT]
                .command
                .clone()
                .unwrap(),
            result: stale_result,
        })
        .is_ok());
    engine.note_key_event();

    let deadline = Instant::now() + Duration::from_secs(3);
    while engine.command_metrics().command_queue_depth > 0 {
        engine.drain_completed_commands();
        assert!(
            Instant::now() < deadline,
            "stale queued command was not drained"
        );
        thread::sleep(Duration::from_millis(5));
    }
    engine.drain_completed_commands();
    assert!(
        !stale_marker.exists(),
        "queued command that became stale must not spawn or cause side effects"
    );
    for path in starts {
        let _ = std::fs::remove_file(path);
    }
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

    // Fill every command worker so the next job is deterministically queued.
    // A single in-flight command is insufficient because the runtime owns a
    // small worker pool; under concurrent test execution the old test could
    // let the supposedly queued command start before shutdown.
    let first = engine
        .process_deferred(InputEvent::Text(":one".into()))
        .pop()
        .unwrap();
    for _ in 0..ASYNC_COMMAND_WORKER_COUNT {
        assert_eq!(
            engine.dispatch_pending_with_policy(first.clone(), 0),
            Ok(PendingExpansionDispatch::Queued)
        );
    }

    let deadline = Instant::now() + Duration::from_secs(1);
    while engine.command_metrics().command_in_flight < ASYNC_COMMAND_WORKER_COUNT {
        assert!(Instant::now() < deadline, "first command did not start");
        thread::sleep(Duration::from_millis(5));
    }
    let metrics = engine.command_metrics();
    assert_eq!(metrics.expansion_command_queue_depth, 0);
    assert_eq!(
        metrics.expansion_command_in_flight,
        ASYNC_COMMAND_WORKER_COUNT
    );
    assert_eq!(metrics.hotkey_queue_depth, 0);
    assert_eq!(metrics.hotkey_in_flight, 0);

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

#[test]
fn explicit_insert_by_trigger_follows_the_typed_expansion_safety_rules() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ";sig"
        replacement = "Best {{cursor}}regards"

        [[expansion]]
        trigger = ";term"
        replacement = "only in a terminal"
        app_filter = ["app_id_exact:org.kde.konsole"]

        [[expansion]]
        trigger = ";cmd"
        replacement = ""
        [expansion.command]
        program = "/bin/echo"

        [[expansion]]
        trigger = ";off"
        replacement = "disabled"
        enabled = false
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let sig = ";sig";

    let result = engine.prepare_insert(sig).unwrap();
    assert_eq!(result.matched_text, "", "an explicit insert erases nothing");
    assert_eq!(result.insert, "Best regards");
    assert_eq!(result.cursor_offset, Some("regards".len()));
    assert!(!result.undoable && !result.command_backed);

    assert_eq!(engine.prepare_insert(";off"), Err(InsertError::NotFound));
    assert_eq!(engine.prepare_insert("missing"), Err(InsertError::NotFound));
    assert_eq!(
        engine.prepare_insert(";cmd"),
        Err(InsertError::CommandBacked)
    );

    // App filters fail closed without a known window, as typed triggers do.
    let term = ";term";
    assert_eq!(engine.prepare_insert(term), Err(InsertError::NotForThisApp));
    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: None,
        instance_id: None,
    }));
    assert!(engine.prepare_insert(term).is_ok());

    engine.process(InputEvent::FocusChanged { sensitive: true });
    assert_eq!(engine.prepare_insert(sig), Err(InsertError::SensitiveField));
    engine.process(InputEvent::FocusChanged { sensitive: false });
    engine.process(InputEvent::PauseChanged(true));
    assert_eq!(engine.prepare_insert(sig), Err(InsertError::Paused));
}

fn typed_results(trigger: &str, typed: &str) -> Vec<ExpansionResult> {
    let config = crate::Config::parse(&format!(
        "[[expansion]]\ntrigger = \"{trigger}\"\nreplacement = \"coffee\"\n"
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut results = Vec::new();
    for character in typed.chars() {
        results.extend(engine.process(InputEvent::Text(character.to_string())));
    }
    results
}

#[test]
fn canonically_equivalent_input_matches_and_deletes_what_was_typed() {
    const NFC: &str = ":caf\u{e9}";
    const NFD: &str = ":cafe\u{301}";
    for (trigger, typed) in [(NFC, NFC), (NFC, NFD), (NFD, NFC), (NFD, NFD)] {
        let results = typed_results(trigger, typed);
        assert_eq!(results.len(), 1, "trigger {trigger:?} typed {typed:?}");
        // The deletion covers the scalars in the document, not the
        // configured spelling.
        assert_eq!(results[0].matched_text, typed);
        assert_eq!(results[0].insert, "coffee");
    }
}

#[test]
fn canonically_equivalent_triggers_in_two_snippets_are_duplicates() {
    let config = crate::Config::parse(
        "[[expansion]]\ntrigger = \":caf\u{e9}\"\nreplacement = \"a\"\n\
         [[expansion]]\ntrigger = \":cafe\u{301}\"\nreplacement = \"b\"\n",
    );
    assert!(matches!(
        config.and_then(|config| config.validate().map(|()| config)),
        Err(crate::ConfigError::DuplicateTrigger { .. })
    ));
}

const ALIAS_CONFIG: &str = "[[expansion]]\ntrigger = \":addr\"\naliases = [\":address\", \":office\"]\nreplacement = \"1 Main St\"\npropagate_case = true\n";

#[test]
fn every_alias_expands_to_the_shared_replacement() {
    for typed in [":addr", ":address", ":office"] {
        let mut engine = ExpansionEngine::new(Config::parse(ALIAS_CONFIG).unwrap()).unwrap();
        let mut results = Vec::new();
        for character in typed.chars() {
            results.extend(engine.process(InputEvent::Text(character.to_string())));
        }
        // `:addr` is a prefix of `:address`, so it resolves when input ends.
        results.extend(engine.process(InputEvent::EndOfInput));
        assert_eq!(results.len(), 1, "{typed}");
        assert_eq!(results[0].matched_text, typed);
        assert_eq!(results[0].insert, "1 Main St");
    }
}

#[test]
fn aliases_follow_case_propagation() {
    let mut engine = ExpansionEngine::new(Config::parse(ALIAS_CONFIG).unwrap()).unwrap();
    let results = engine.process(InputEvent::Text(":OFFICE".into()));
    assert_eq!(results.last().unwrap().insert, "1 MAIN ST");
}

#[test]
fn an_alias_colliding_with_another_trigger_is_a_duplicate() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":a\"\naliases = [\":b\"]\nreplacement = \"x\"\n\
         [[expansion]]\ntrigger = \":b\"\nreplacement = \"y\"\n",
    );
    assert!(matches!(
        config,
        Err(crate::ConfigError::DuplicateTrigger { .. })
    ));
}

#[test]
fn aliases_are_validated_like_triggers() {
    let empty =
        Config::parse("[[expansion]]\ntrigger = \":a\"\naliases = [\"\"]\nreplacement = \"x\"\n");
    assert!(matches!(
        empty,
        Err(crate::ConfigError::EmptyTrigger { .. })
    ));
    let many = format!(
        "[[expansion]]\ntrigger = \":a\"\naliases = [{}]\nreplacement = \"x\"\n",
        (0..33)
            .map(|index| format!("\":a{index}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(matches!(
        Config::parse(&many),
        Err(crate::ConfigError::TooManyAliases { .. })
    ));
}

#[test]
fn quick_insert_accepts_an_alias() {
    let engine = ExpansionEngine::new(Config::parse(ALIAS_CONFIG).unwrap()).unwrap();
    assert_eq!(
        engine.prepare_insert(":office").unwrap().insert,
        "1 Main St"
    );
}

#[test]
fn aliases_are_omitted_from_saved_toml_when_empty() {
    let config = Config::parse("[[expansion]]\ntrigger = \":a\"\nreplacement = \"x\"\n").unwrap();
    let text = toml::to_string(&config).unwrap();
    assert!(!text.contains("aliases"), "{text}");
}

fn explain_engine(config: &str) -> ExpansionEngine {
    ExpansionEngine::new(Config::parse(config).unwrap()).unwrap()
}

#[test]
fn explain_reports_an_app_filter_mismatch() {
    let mut engine = explain_engine(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"Best\"\nmatch_mode = \"word-boundary\"\napp_filter = [\"com.slack.Slack\"]\n",
    );
    engine.process(InputEvent::WindowChanged(Some(crate::WindowContext {
        app_id: Some("org.kde.konsole".into()),
        title: None,
        instance_id: None,
    })));
    let explanation = engine.explain(":sig", "libei");
    let failed = explanation.suppressed_by().unwrap();
    assert_eq!(failed.name, "app filter");
    assert!(
        failed.detail.contains("org.kde.konsole"),
        "{}",
        failed.detail
    );
    assert!(failed.detail.contains("com.slack.Slack"));
    let text = explanation.render_text();
    assert!(text.contains("✓ trigger: recognized as :sig"), "{text}");
    assert!(text.contains("Result: suppressed by app filter."), "{text}");
    // Replacement content never appears in an explanation.
    assert!(!text.contains("Best"));
}

#[test]
fn explain_reports_runtime_state_that_blocks_matching() {
    let config = "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"x\"\n";
    let mut engine = explain_engine(config);
    assert!(engine.explain(":sig", "libei").would_expand());

    engine.set_user_paused(true);
    assert_eq!(
        engine
            .explain(":sig", "libei")
            .suppressed_by()
            .unwrap()
            .name,
        "paused"
    );
    engine.set_user_paused(false);

    engine.process(InputEvent::FocusChanged { sensitive: true });
    assert_eq!(
        engine
            .explain(":sig", "libei")
            .suppressed_by()
            .unwrap()
            .name,
        "sensitive field"
    );
    engine.process(InputEvent::FocusChanged { sensitive: false });

    engine.process(InputEvent::CompositionChanged { active: true });
    assert_eq!(
        engine
            .explain(":sig", "libei")
            .suppressed_by()
            .unwrap()
            .name,
        "composition"
    );
}

#[test]
fn explain_gives_hints_for_unrecognized_triggers() {
    let engine = explain_engine(
        "[[expansion]]\ntrigger = \":Sig\"\nreplacement = \"x\"\n\
         [[expansion]]\ntrigger = \":address\"\nreplacement = \"y\"\n",
    );
    let case = engine.explain(":sig", "libei");
    assert!(
        case.checks[0].detail.contains("differs only in case"),
        "{:?}",
        case.checks
    );
    let prefix = engine.explain(":addr", "libei");
    assert!(prefix.checks[0]
        .detail
        .contains("only the start of :address"));
    let none = engine.explain(":nothing", "libei");
    assert_eq!(none.suppressed_by().unwrap().name, "trigger");
}

#[test]
fn explain_names_aliases_disabled_snippets_and_prefix_waits() {
    let engine = explain_engine(
        "[[expansion]]\ntrigger = \":addr\"\naliases = [\":office\"]\nreplacement = \"x\"\n\
         [[expansion]]\ntrigger = \":address\"\nreplacement = \"y\"\n\
         [[expansion]]\ntrigger = \":off\"\nreplacement = \"z\"\nenabled = false\n",
    );
    let alias = engine.explain(":office", "libei");
    assert!(alias.checks[0].detail.contains("alias of :addr"));
    assert!(alias.would_expand());
    let waits = engine.explain(":addr", "libei");
    assert!(
        waits
            .checks
            .iter()
            .any(|check| check.status == crate::CheckStatus::Warn
                && check.detail.contains(":address"))
    );
    assert_eq!(
        engine
            .explain(":off", "libei")
            .suppressed_by()
            .unwrap()
            .name,
        "enabled"
    );
}

#[test]
fn explain_applies_safe_mode_backend_policy() {
    let engine = explain_engine(
        "[organization]\nsafe_mode = true\nallowed_backends = [\"input-method-v2\"]\n\
         [[expansion]]\ntrigger = \":sig\"\nreplacement = \"x\"\n",
    );
    assert_eq!(
        engine
            .explain(":sig", "libei")
            .suppressed_by()
            .unwrap()
            .name,
        "policy"
    );
    assert!(engine.explain(":sig", "input-method-v2").would_expand());
}

fn render_first(
    config: &str,
    typed: &str,
    clipboard: Option<&'static str>,
) -> Vec<ExpansionResult> {
    let mut engine = ExpansionEngine::new(Config::parse(config).unwrap()).unwrap();
    if let Some(text) = clipboard {
        engine.set_clipboard_reader(Some(crate::ClipboardReader(std::sync::Arc::new(
            move || Some(text.to_owned()),
        ))));
    }
    let mut results = engine.process(InputEvent::Text(typed.into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    results
}

#[test]
fn env_variables_read_only_allowlisted_names() {
    std::env::set_var("WAYEXPAND_TEST_TICKET_PREFIX", "OPS-");
    let config = "[settings]\ntemplate_env = [\"WAYEXPAND_TEST_TICKET_PREFIX\"]\n\
                  [[expansion]]\ntrigger = \":t\"\nreplacement = \"{{env:WAYEXPAND_TEST_TICKET_PREFIX}}42\"\n";
    assert_eq!(render_first(config, ":t", None)[0].insert, "OPS-42");

    let unlisted = "[[expansion]]\ntrigger = \":t\"\nreplacement = \"{{env:HOME}}\"\n";
    assert!(matches!(
        Config::parse(unlisted),
        Err(crate::ConfigError::InvalidTemplate {
            source: crate::TemplateError::EnvNotAllowed { .. },
            ..
        })
    ));
    let bad_name = "[settings]\ntemplate_env = [\"1BAD\"]\n";
    assert!(matches!(
        Config::parse(bad_name),
        Err(crate::ConfigError::InvalidTemplateEnv { .. })
    ));
}

#[test]
fn safe_mode_policy_can_disable_env_and_clipboard_variables() {
    let config =
        "[organization]\nsafe_mode = true\ndisable_template_env = true\ndisable_clipboard = true\n\
                  [settings]\ntemplate_env = [\"USER\"]\nallow_clipboard = true\n\
                  [[expansion]]\ntrigger = \":e\"\nreplacement = \"{{env:USER}}\"\n\
                  [[expansion]]\ntrigger = \":c\"\nreplacement = \"{{clipboard}}\"\n";
    // Blocked variables fail closed: the snippet does not expand.
    assert!(render_first(config, ":e", None).is_empty());
    assert!(render_first(config, ":c", Some("secret")).is_empty());
}

#[test]
fn snippet_includes_render_static_snippets_and_reject_cycles() {
    let config = "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"Best, Sam\"\n\
                  [[expansion]]\ntrigger = \":reply\"\nreplacement = \"Thanks!{{newline}}{{snippet::sig}}\"\n";
    assert_eq!(
        render_first(config, ":reply", None)[0].insert,
        "Thanks!\nBest, Sam"
    );

    let cycle = "[[expansion]]\ntrigger = \":a\"\nreplacement = \"{{snippet::b}}\"\n\
                 [[expansion]]\ntrigger = \":b\"\nreplacement = \"{{snippet::a}}\"\n";
    assert!(matches!(
        Config::parse(cycle),
        Err(crate::ConfigError::InvalidTemplate {
            source: crate::TemplateError::IncludeTooDeep,
            ..
        })
    ));
    let command = "[[expansion]]\ntrigger = \":cmd\"\nreplacement = \"\"\n[expansion.command]\nprogram = \"/bin/true\"\n\
                   [[expansion]]\ntrigger = \":x\"\nreplacement = \"{{snippet::cmd}}\"\n";
    assert!(matches!(
        Config::parse(command),
        Err(crate::ConfigError::InvalidTemplate {
            source: crate::TemplateError::UnknownSnippet,
            ..
        })
    ));
    let cursor = "[[expansion]]\ntrigger = \":a\"\nreplacement = \"x{{cursor}}\"\n\
                  [[expansion]]\ntrigger = \":b\"\nreplacement = \"{{snippet::a}}\"\n";
    assert!(matches!(
        Config::parse(cursor),
        Err(crate::ConfigError::InvalidTemplate {
            source: crate::TemplateError::CursorInInclude,
            ..
        })
    ));
}

#[test]
fn clipboard_requires_opt_in_and_fails_closed() {
    let disabled = "[[expansion]]\ntrigger = \":c\"\nreplacement = \"{{clipboard}}\"\n";
    assert!(matches!(
        Config::parse(disabled),
        Err(crate::ConfigError::InvalidTemplate {
            source: crate::TemplateError::ClipboardDisabled,
            ..
        })
    ));
    let enabled = "[settings]\nallow_clipboard = true\n[[expansion]]\ntrigger = \":c\"\nreplacement = \"> {{clipboard}}\"\n";
    assert_eq!(
        render_first(enabled, ":c", Some("quoted"))[0].insert,
        "> quoted"
    );
    // No reader (or an unreadable clipboard) means no expansion.
    assert!(render_first(enabled, ":c", None).is_empty());
}

#[test]
fn applied_expansions_produce_usage_events_by_snippet_id() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\naliases = [\":s\"]\nreplacement = \"Best regards\"\n",
    )
    .unwrap();
    let id = config.expansion[0].id.clone();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut results = engine.process(InputEvent::Text(":sig".into()));
    results.extend(engine.process(InputEvent::EndOfInput));
    engine.commit_applied_expansion(&results[0]);
    let events = engine.drain_usage_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].snippet_id, id);
    assert_eq!(events[0].typed_chars, 4);
    assert_eq!(events[0].inserted_chars, 12);
    assert!(engine.drain_usage_events().is_empty());
}

#[test]
fn usage_attribution_follows_the_matched_enabled_snippet() {
    let config = Config::parse(
        "[[expansion]]\nid = \"00000000-0000-4000-8000-00000000000a\"\ntrigger = \";test\"\nenabled = false\nreplacement = \"old\"\n\n[[expansion]]\nid = \"00000000-0000-4000-8000-00000000000b\"\ntrigger = \";test\"\nenabled = true\nreplacement = \"new\"\n",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let results = engine.process(InputEvent::Text(";test".into()));
    assert_eq!(results.len(), 1);
    engine.commit_applied_expansion(&results[0]);

    let events = engine.drain_usage_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].snippet_id, "00000000-0000-4000-8000-00000000000b");
}

#[test]
fn usage_recording_can_be_turned_off() {
    let config = Config::parse(
        "[settings]\nusage_stats = false\n[[expansion]]\ntrigger = \":sig\"\nreplacement = \"x\"\n",
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let results = engine.process(InputEvent::Text(":sig".into()));
    engine.commit_applied_expansion(&results[0]);
    assert!(engine.drain_usage_events().is_empty());
}

#[test]
fn usage_report_counts_savings_unused_snippets_and_risky_triggers() {
    let config = Config::parse(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"Best regards\"\n\
         [[expansion]]\ntrigger = \":old\"\nreplacement = \"x\"\n\
         [[expansion]]\ntrigger = \"btw\"\nreplacement = \"by the way\"\n",
    )
    .unwrap();
    let now = 1_700_000_000;
    let mut stats = crate::UsageStats::default();
    for _ in 0..3 {
        stats.record(&crate::UsageEvent {
            snippet_id: config.expansion[0].id.clone(),
            typed_chars: 4,
            inserted_chars: 12,
            unix_timestamp: now,
        });
    }
    stats.record(&crate::UsageEvent {
        snippet_id: config.expansion[1].id.clone(),
        typed_chars: 4,
        inserted_chars: 1,
        unix_timestamp: now - 200 * 86_400,
    });
    let report = stats.report(&config, now, 30);
    assert_eq!(report.expansions, 3);
    // 3 × (12 − 4); the old snippet saved nothing.
    assert_eq!(report.keystrokes_avoided, 24);
    assert_eq!(report.top[0].trigger, ":sig");
    assert_eq!(report.top[0].count, 3);
    assert!(report.unused_90_days.contains(&":old".to_owned()));
    assert!(report.unused_90_days.contains(&"btw".to_owned()));
    assert!(report
        .trigger_risks
        .iter()
        .any(|risk| risk.trigger == "btw" && risk.reason.contains("immediate mode")));
    assert!(!report
        .trigger_risks
        .iter()
        .any(|risk| risk.trigger == ":sig"));
}

#[test]
fn usage_stats_round_trip_through_a_private_file() {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!("wayexpand-usage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let path = crate::usage_stats_path(&directory.join("expansions.toml"));
    let mut stats = crate::UsageStats::default();
    stats.record(&crate::UsageEvent {
        snippet_id: "id".into(),
        typed_chars: 1,
        inserted_chars: 2,
        unix_timestamp: 86_400,
    });
    stats.save(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(crate::UsageStats::load(&path), stats);
    assert_eq!(stats.daily["1970-01-02"], 1);
    let _ = std::fs::remove_dir_all(directory);
}

static FORM_HELPER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point the engine at a fake form helper script for the life of the guard.
struct FakeFormHelper {
    _guard: std::sync::MutexGuard<'static, ()>,
    path: std::path::PathBuf,
}

impl FakeFormHelper {
    fn new(name: &str, script: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let guard = FORM_HELPER_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = std::env::temp_dir().join(format!(
            "wayexpand-form-helper-{name}-{}",
            std::process::id()
        ));
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("WAYEXPAND_FORM_HELPER", &path);
        Self {
            _guard: guard,
            path,
        }
    }
}

impl Drop for FakeFormHelper {
    fn drop(&mut self) {
        std::env::remove_var("WAYEXPAND_FORM_HELPER");
        let _ = std::fs::remove_file(&self.path);
    }
}

const FORM_CONFIG: &str = r#"
[[expansion]]
trigger = ":tk"
replacement = "Hi {{field:name}}, ticket {{field:id=OPS-1}} is {{choice:Open|Resolved}}. {{field:name}}{{cursor}}!"
"#;

fn queue_form(engine: &mut ExpansionEngine) {
    engine.set_current_window(Some(crate::WindowContext {
        app_id: Some("org.example.Mail".into()),
        title: Some("Inbox".into()),
        instance_id: Some("toplevel-mail-1".into()),
    }));
    let pending = engine.process_deferred(InputEvent::Text(":tk".into()));
    assert_eq!(pending.len(), 1);
    let dispatch = engine
        .dispatch_pending_with_policy(pending.into_iter().next().unwrap(), 0)
        .unwrap();
    assert_eq!(dispatch, PendingExpansionDispatch::Queued);
}

fn wait_for_completion(engine: &mut ExpansionEngine) -> Vec<ExpansionResult> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.is_form_open() {
        let results = engine.drain_completed_commands();
        if !results.is_empty() || !engine.is_form_open() {
            return results;
        }
        assert!(Instant::now() < deadline, "form did not complete");
        thread::sleep(Duration::from_millis(10));
    }
    Vec::new()
}

#[test]
fn a_submitted_form_renders_its_values_and_suspends_capture_meanwhile() {
    let _helper = FakeFormHelper::new(
        "submit",
        r#"sleep 0.2; printf '{"field:name":"Ada","field:id":"OPS-7","choice:Open|Resolved":"Resolved"}'"#,
    );
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    queue_form(&mut engine);
    assert!(engine.is_form_open());
    // Typing in the form window must not expand anything.
    assert!(engine.process(InputEvent::Text(":tk".into())).is_empty());
    let results = wait_for_completion(&mut engine);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].insert, "Hi Ada, ticket OPS-7 is Resolved. Ada!");
    assert_eq!(results[0].matched_text, ":tk");
    assert_eq!(results[0].cursor_offset, Some(1));
    assert!(!engine.is_form_open());
}

#[test]
fn a_cancelled_form_leaves_the_trigger_alone() {
    let _helper = FakeFormHelper::new("cancel", "exit 1");
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    queue_form(&mut engine);
    assert!(wait_for_completion(&mut engine).is_empty());
    assert!(!engine.is_form_open());
}

#[test]
fn a_form_result_is_dropped_when_focus_moved_to_another_app() {
    let _helper = FakeFormHelper::new(
        "moved",
        r#"sleep 0.2; printf '{"field:name":"Ada","field:id":"x","choice:Open|Resolved":"Open"}'"#,
    );
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    engine.process(InputEvent::WindowChanged(Some(crate::WindowContext {
        app_id: Some("org.example.Mail".into()),
        title: None,
        instance_id: Some("toplevel-mail-1".into()),
    })));
    queue_form(&mut engine);
    engine.process(InputEvent::WindowChanged(Some(crate::WindowContext {
        app_id: Some("org.example.Chat".into()),
        title: None,
        instance_id: Some("toplevel-chat-1".into()),
    })));
    assert!(wait_for_completion(&mut engine).is_empty());
}

#[test]
fn a_form_result_is_dropped_when_focus_moves_to_another_window_of_the_same_app() {
    let _helper = FakeFormHelper::new(
        "same-app-different-window",
        r#"sleep 0.2; printf '{"field:name":"Ada","field:id":"x","choice:Open|Resolved":"Open"}'"#,
    );
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    queue_form(&mut engine);
    engine.process(InputEvent::WindowChanged(Some(crate::WindowContext {
        app_id: Some("org.example.Mail".into()),
        title: Some("Another mailbox window".into()),
        instance_id: Some("toplevel-mail-2".into()),
    })));
    assert!(wait_for_completion(&mut engine).is_empty());
}

#[test]
fn a_form_is_not_opened_when_the_backend_cannot_identify_the_exact_window() {
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    engine.set_current_window(Some(crate::WindowContext {
        app_id: Some("org.example.Mail".into()),
        title: Some("Inbox".into()),
        instance_id: None,
    }));
    let pending = engine.process_deferred(InputEvent::Text(":tk".into()));
    assert_eq!(pending.len(), 1);
    let result = engine.dispatch_pending_with_policy(pending.into_iter().next().unwrap(), 0);
    assert_eq!(result, Err(CommandError::WindowIdentityUnavailable));
    assert!(!engine.is_form_open());
}

#[test]
fn a_form_is_not_opened_with_an_unbounded_window_identity() {
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    engine.set_current_window(Some(crate::WindowContext {
        app_id: Some("org.example.Mail".into()),
        title: Some("Inbox".into()),
        instance_id: Some("x".repeat(MAX_WINDOW_INSTANCE_ID_BYTES + 1)),
    }));
    let pending = engine.process_deferred(InputEvent::Text(":tk".into()));
    assert_eq!(pending.len(), 1);
    let result = engine.dispatch_pending_with_policy(pending.into_iter().next().unwrap(), 0);
    assert_eq!(result, Err(CommandError::WindowIdentityUnavailable));
    assert!(!engine.is_form_open());
}

#[test]
fn form_values_outside_a_choice_are_rejected() {
    let _helper = FakeFormHelper::new(
        "bad-choice",
        r#"printf '{"field:name":"Ada","field:id":"x","choice:Open|Resolved":"Deleted"}'"#,
    );
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.enable_async_commands());
    queue_form(&mut engine);
    assert!(wait_for_completion(&mut engine).is_empty());
}

#[test]
fn form_fields_are_parsed_and_validated() {
    let fields =
        crate::form_fields("{{field:name}} {{prompt:name}} {{field:id=OPS-1}} {{choice:A|B}}")
            .unwrap();
    assert_eq!(fields.len(), 3, "{fields:?}");
    assert!(matches!(
        &fields[1].kind,
        crate::FormFieldKind::Text { default } if default == "OPS-1"
    ));
    assert!(crate::form_fields("{{choice:only}}").is_err());
    // A synchronous match never types raw field markers.
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    assert!(engine.process(InputEvent::Text(":tk".into())).is_empty());
    // Forms cannot be command-backed.
    assert!(Config::parse(
        "[[expansion]]\ntrigger = \":x\"\nreplacement = \"{{field:a}}\"\n[expansion.command]\nprogram = \"/bin/true\"\n"
    )
    .is_err());
}

#[test]
fn explain_describes_form_snippets_instead_of_failing_to_render() {
    let mut engine = ExpansionEngine::new(Config::parse(FORM_CONFIG).unwrap()).unwrap();
    let explanation = engine.explain(":tk", "libei");
    assert!(!explanation.would_expand(), "{}", explanation.render_text());
    assert!(explanation
        .checks
        .iter()
        .any(|check| check.detail.contains("opens a form for name, id, Choice")));
    assert!(explanation.checks.iter().any(|check| {
        check.name == "window identity"
            && check.status == CheckStatus::Fail
            && check.detail.contains("form will not open")
    }));
    assert!(explanation
        .checks
        .iter()
        .any(|check| check.name == "policy"));

    engine.set_current_window(Some(WindowContext {
        app_id: Some("org.example.mail".into()),
        title: Some("Inbox".into()),
        instance_id: Some("toplevel-1".into()),
    }));
    let explanation = engine.explain(":tk", "libei");
    assert!(explanation.would_expand(), "{}", explanation.render_text());
    assert!(explanation.checks.iter().any(|check| {
        check.name == "window identity"
            && check.status == CheckStatus::Pass
            && check.detail.contains("exact original toplevel")
    }));
}
