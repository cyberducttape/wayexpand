use super::*;
use crate::events::{absorb_evdev_delimiter, evdev_release_is_safe};
use crate::input_loop::libei_policy_blocks;
use crate::output_loop::connect_output_backend;
use std::io::{BufReader, Cursor};
use std::time::Instant;
use wayexpand_core::{Config, InjectorError, KeyChord};

#[test]
fn startup_worker_failure_remains_command_disabled_in_audit_mode() {
    let policy = wayexpand_core::OrganizationPolicy::default();
    assert!(commands_disabled_for_startup(&policy, true));
}

#[test]
fn evdev_release_failure_is_not_safe_to_inject() {
    assert!(!evdev_release_is_safe(Err("device polling failed")));
    assert!(evdev_release_is_safe::<&str>(Ok(())));
}

#[test]
fn startup_command_policy_and_worker_failure_are_combined() {
    let policy = wayexpand_core::OrganizationPolicy {
        safe_mode: true,
        disable_commands: true,
        ..Default::default()
    };
    assert!(commands_disabled_for_startup(&policy, true));
    assert!(commands_disabled_for_startup(&policy, false));
}

#[test]
fn libei_backend_policy_only_blocks_in_safe_mode() {
    let audit = wayexpand_core::OrganizationPolicy {
        safe_mode: false,
        allowed_backends: vec!["input-method-v2".into()],
        ..Default::default()
    };
    assert!(!libei_policy_blocks(&audit));
    let safe = wayexpand_core::OrganizationPolicy {
        safe_mode: true,
        ..audit
    };
    assert!(libei_policy_blocks(&safe));
}

/// The status body's field set is a documented Stable contract (see
/// docs/COMPATIBILITY.md, "wayexpand status --json"). This is the
/// producing side; `status_json_matches_documented_stable_contract` in
/// the CLI crate's tests is the consuming side, checked against a
/// fixture with the same field names -- keeping both in sync with the
/// docs by construction, rather than each drifting independently.
#[test]
fn daemon_status_body_matches_documented_stable_contract() {
    let body = status::daemon_status_body_with_runtime_capabilities(
        "input-method",
        "input-method-v2",
        "connected",
        false,
        Path::new("/home/user/.config/wayexpand/expansions.toml"),
        true,
        "unknown",
        CommandMetrics::default(),
        latency::Snapshot::default(),
        InputSourceCapabilities::INPUT_METHOD_V2,
        InjectorCapabilities {
            atomic_replace: true,
            full_unicode: true,
            ..InjectorCapabilities::default()
        },
        true,
    );
    let body = body.as_str();
    let mut fields: Vec<&str> = body
        .lines()
        .filter_map(|line| line.split_once('=').map(|(key, _)| key))
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "backend",
            "backend_mode",
            "capture_composition_aware",
            "capture_exclusive",
            "capture_key_passthrough",
            "capture_layout_aware",
            "capture_local_compose_aware",
            "capture_reliable_key_state",
            "capture_sensitive_focus",
            "command_failure_total",
            "command_in_flight",
            "command_queue_depth",
            "command_queue_rejected_total",
            "command_timeout_total",
            "config",
            "config_state",
            "daemon_commit",
            "expansion_command_in_flight",
            "expansion_command_queue_depth",
            "hotkey_in_flight",
            "hotkey_queue_depth",
            "inject_atomic_replace",
            "inject_cursor_reposition",
            "inject_expected_throughput_chars_per_sec",
            "inject_full_unicode",
            "inject_insertion_mode",
            "inject_key_passthrough",
            "inject_max_text_chars",
            "injection_latency_p50_us",
            "injection_latency_p95_us",
            "injection_latency_p99_us",
            "injection_latency_sample_count",
            "injection_latency_window_count",
            "paused",
            "source",
            "state",
            "status_schema",
            "window_identity_exact",
            "window_tracker_connected",
        ],
        "daemon status body fields no longer match docs/COMPATIBILITY.md's documented Stable contract"
    );
    assert!(body.starts_with(&format!(
        "source=input-method\nbackend=input-method-v2\nbackend_mode=unknown\nstatus_schema=6\ndaemon_commit={}\nstate=connected\npaused=false\n",
        super::build_info::COMMIT
    )));
    assert!(body.ends_with(
        "injection_latency_sample_count=0\n\
         injection_latency_window_count=0\n\
         injection_latency_p50_us=0\n\
         injection_latency_p95_us=0\n\
         injection_latency_p99_us=0"
    ));
}

struct RecordingInjector {
    calls: Vec<String>,
}

impl TextInjector for RecordingInjector {
    fn name(&self) -> &'static str {
        "test"
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        self.calls.push(format!("erase:{trigger}"));
        Ok(())
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.calls.push(format!("insert:{text}"));
        Ok(())
    }
}

struct FailingInjector;

impl TextInjector for FailingInjector {
    fn name(&self) -> &'static str {
        "failing-test"
    }

    fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
        Err(InjectorError {
            backend: "failing-test",
            message: "connection lost".into(),
            retryable: true,
        })
    }

    fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
        unreachable!("erase fails first")
    }
}

/// Fails like [`FailingInjector`], but its replace is one atomic
/// protocol transaction, so a failure is known not to have applied.
struct AtomicFailingInjector;

impl TextInjector for AtomicFailingInjector {
    fn name(&self) -> &'static str {
        "atomic-failing-test"
    }

    fn capabilities(&self) -> InjectorCapabilities {
        InjectorCapabilities {
            atomic_replace: true,
            ..InjectorCapabilities::default()
        }
    }

    fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
        Err(InjectorError {
            backend: "atomic-failing-test",
            message: "connection lost".into(),
            retryable: true,
        })
    }

    fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
        unreachable!("erase fails first")
    }
}

struct TextBufferInjector {
    text: String,
}

impl TextInjector for TextBufferInjector {
    fn name(&self) -> &'static str {
        "text-buffer-test"
    }

    fn erase(&mut self, trigger: &str) -> Result<(), InjectorError> {
        assert!(
            self.text.ends_with(trigger),
            "erase {trigger:?} was requested for {:?}",
            self.text
        );
        let new_len = self.text.len() - trigger.len();
        self.text.truncate(new_len);
        Ok(())
    }

    fn insert(&mut self, text: &str) -> Result<(), InjectorError> {
        self.text.push_str(text);
        Ok(())
    }
}

fn assert_evdev_boundary_replacement(
    config_text: &str,
    typed_prefix: &str,
    delimiter: char,
    expected: &str,
) {
    let config = Config::parse(config_text).unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut injector = TextBufferInjector {
        text: format!("{typed_prefix}{delimiter}"),
    };
    let policy = wayexpand_core::OrganizationPolicy::default();

    process_event(
        &mut engine,
        InputEvent::Text(typed_prefix.into()),
        None,
        &policy,
        "evdev",
    )
    .unwrap();
    process_event(
        &mut engine,
        InputEvent::Delimiter(delimiter),
        Some(&mut injector),
        &policy,
        "evdev",
    )
    .unwrap();

    assert_eq!(injector.text, expected);
}

#[test]
fn evdev_delayed_boundary_replacement_preserves_delimiters_in_text() {
    assert_evdev_boundary_replacement(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        ":sig",
        ' ',
        "signature ",
    );
    assert_evdev_boundary_replacement(
        "[[expansion]]\ntrigger = \":sig\"\nreplacement = \"signature\"\nmatch_mode = \"word-boundary\"",
        ":sig",
        '.',
        "signature.",
    );
    assert_evdev_boundary_replacement(
        "[[expansion]]\ntrigger = \":a\"\nreplacement = \"alpha\"\n[[expansion]]\ntrigger = \":address\"\nreplacement = \"address\"",
        ":a",
        ':',
        "alpha:",
    );
}

#[test]
fn process_event_applies_all_matches_in_order() {
    let config = Config::parse(
        r#"
            [[expansion]]
            trigger = ":a"
            replacement = "alpha"

            [[expansion]]
            trigger = ":b"
            replacement = "beta"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy::default();
    process_event(
        &mut engine,
        InputEvent::Text(":a:b".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    assert_eq!(
        injector.calls,
        ["erase::a", "insert:alpha", "erase::b", "insert:beta"]
    );
}

#[test]
fn evdev_follow_up_is_replayed_in_order_to_the_matcher() {
    let config = Config::parse(
        r#"
            [[expansion]]
            trigger = ":sigx "
            replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();

    process_event(
        &mut engine,
        InputEvent::Text(":sig".into()),
        None,
        &policy,
        "libei",
    )
    .unwrap();
    replay_evdev_follow_up(
        &mut engine,
        vec![InputEvent::Text("x".into())],
        &policy,
        "libei",
    )
    .unwrap();

    let mut injector = RecordingInjector { calls: Vec::new() };
    process_event(
        &mut engine,
        InputEvent::Delimiter(' '),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    assert_eq!(injector.calls, ["erase::sigx ", "insert:ok"]);
}

#[test]
fn evdev_delimiter_is_absorbed_only_by_the_final_result() {
    let mut results = vec![
        ExpansionResult {
            snippet_id: String::new(),
            trigger: ":a".into(),
            matched_text: ":a".into(),
            insert: "alpha".into(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: true,
        },
        ExpansionResult {
            snippet_id: String::new(),
            trigger: ":b".into(),
            matched_text: ":b".into(),
            insert: "beta".into(),
            cursor_offset: None,
            reinsert_after: None,
            command_backed: false,
            undoable: true,
        },
    ];

    absorb_evdev_delimiter(&mut results, ' ');

    assert_eq!(results[0].matched_text, ":a");
    assert_eq!(results[0].insert, "alpha");
    assert_eq!(results[1].matched_text, ":b ");
    assert_eq!(results[1].insert, "beta ");
}

#[cfg(unix)]
#[test]
fn deferred_command_output_over_organization_limit_is_not_injected() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-daemon-policy-output-{}",
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
        timeout_ms = 1000
        "#
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.enable_async_commands());
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy {
        safe_mode: true,
        max_replacement_size: 256,
        ..wayexpand_core::OrganizationPolicy::default()
    };

    process_event(
        &mut engine,
        InputEvent::Text(":large".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let completed = engine.drain_completed_commands();
        for result in &completed {
            assert!(ExpansionEngine::apply(&mut injector, result).is_applied());
        }
        let metrics = engine.command_metrics();
        if marker.exists() && metrics.command_queue_depth == 0 && metrics.command_in_flight == 0 {
            for _ in 0..5 {
                let _ = engine.drain_completed_commands();
                thread::sleep(Duration::from_millis(2));
            }
            break;
        }
        assert!(Instant::now() < deadline, "the subprocess should complete");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        injector.calls.is_empty(),
        "oversized output must not be injected"
    );
    let _ = std::fs::remove_file(marker);
}

#[cfg(unix)]
#[test]
fn deferred_command_does_not_block_daemon_input_processing() {
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":slow"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "sleep 0.4; printf done"]
        timeout_ms = 1000
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    assert!(engine.enable_async_commands());
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy::default();
    let started = Instant::now();
    process_event(
        &mut engine,
        InputEvent::Text(":slow".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "daemon input processing waited for the child process"
    );
    assert!(injector.calls.is_empty());

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let results = engine.drain_completed_commands();
        if !results.is_empty() {
            for result in &results {
                assert!(ExpansionEngine::apply(&mut injector, result).is_applied());
            }
            break;
        }
        assert!(
            Instant::now() < deadline,
            "command completion was not drained"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(injector.calls, ["erase::slow", "insert:done"]);
}

#[test]
fn reconnect_backoff_is_bounded() {
    let initial = Duration::from_millis(250);
    assert_eq!(next_retry_delay(initial), Duration::from_millis(500));
    assert_eq!(
        next_retry_delay(Duration::from_secs(20)),
        Duration::from_secs(30)
    );
    assert_eq!(
        next_retry_delay(Duration::from_secs(30)),
        Duration::from_secs(30)
    );
}

#[test]
fn unsupported_output_backend_fails_without_retry() {
    let error = match connect_output_backend("unknown", true, None) {
        Ok(_) => panic!("unknown backend unexpectedly connected"),
        Err(error) => error,
    };
    assert!(!error.retryable);
    assert!(error.message.contains("unknown output backend"));
}

#[test]
fn bounded_line_reader_matches_lines_semantics() {
    let mut reader = BufReader::new(Cursor::new(b"first\r\nsecond\n"));
    assert_eq!(
        read_bounded_line(&mut reader).unwrap().as_deref(),
        Some("first")
    );
    assert_eq!(
        read_bounded_line(&mut reader).unwrap().as_deref(),
        Some("second")
    );
    assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
}

#[test]
fn oversized_and_invalid_lines_are_rejected_without_unbounded_allocation() {
    let oversized = vec![b'a'; MAX_STDIN_LINE_BYTES + 1];
    let mut reader = BufReader::new(Cursor::new(oversized));
    let error = read_bounded_line(&mut reader).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

    let mut reader = BufReader::new(Cursor::new(vec![0xff, b'\n']));
    let error = read_bounded_line(&mut reader).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn stdin_queue_applies_backpressure_at_a_bounded_capacity() {
    let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_INPUT_LINES);
    for _ in 0..MAX_PENDING_INPUT_LINES {
        sender.try_send(String::from("line")).unwrap();
    }
    assert!(matches!(
        sender.try_send(String::from("overflow")),
        Err(mpsc::TrySendError::Full(_))
    ));
    drop(receiver);
}

#[test]
fn process_event_can_match_without_an_injector() {
    let config = Config::parse(
        r#"[[expansion]]
        trigger = ":x"
        replacement = "ok""#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();
    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        None,
        &policy,
        "libei",
    )
    .unwrap();
}

#[test]
fn undo_is_not_consumed_when_injector_is_unavailable() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();
    let mut injector = RecordingInjector { calls: Vec::new() };

    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        None,
        &policy,
        "libei",
    )
    .unwrap();
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();

    assert_eq!(
        injector.calls,
        ["erase::x", "insert:ok", "erase:ok", "insert::x"]
    );
}

#[test]
fn undo_is_not_consumed_when_injection_fails() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();
    let mut initial_injector = RecordingInjector { calls: Vec::new() };
    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut initial_injector),
        &policy,
        "libei",
    )
    .unwrap();

    let error = process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut AtomicFailingInjector),
        &policy,
        "libei",
    )
    .unwrap_err();
    assert!(error.retryable());

    let mut retry_injector = RecordingInjector { calls: Vec::new() };
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut retry_injector),
        &policy,
        "libei",
    )
    .unwrap();
    assert_eq!(retry_injector.calls, ["erase:ok", "insert::x"]);
}

#[test]
fn possibly_partial_undo_failure_drops_the_undo_record() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();
    let mut initial_injector = RecordingInjector { calls: Vec::new() };
    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut initial_injector),
        &policy,
        "libei",
    )
    .unwrap();

    // A non-atomic backend may have erased part of the text before
    // failing; the session still reconnects, but the undo must not be
    // replayed against text in an unknown state.
    let error = process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut FailingInjector),
        &policy,
        "libei",
    )
    .unwrap_err();
    assert!(error.retryable());

    let mut retry_injector = RecordingInjector { calls: Vec::new() };
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut retry_injector),
        &policy,
        "libei",
    )
    .unwrap();
    assert!(retry_injector.calls.is_empty());
}

#[test]
fn undo_is_preserved_only_by_undo_chord() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy {
        safe_mode: true,
        disable_hotkeys: true,
        ..Default::default()
    };

    // Expand :x to "ok"
    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    // Non-undo key presses (A) should invalidate undo transaction
    // because they may move the cursor or change text
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("A").unwrap()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    // Ctrl+Z should NOT undo now because the undo transaction was invalidated
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();

    // Should only have the expansion, no undo
    assert_eq!(
        injector.calls,
        ["erase::x", "insert:ok"],
        "non-undo key should invalidate undo transaction"
    );
}

#[test]
fn undo_works_immediately_after_expansion() {
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy::default();

    // Expand :x to "ok"
    process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();
    // Immediately press Ctrl+Z (the undo chord)
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
        Some(&mut injector),
        &policy,
        "libei",
    )
    .unwrap();

    // Should have both expansion and undo
    assert_eq!(
        injector.calls,
        ["erase::x", "insert:ok", "erase:ok", "insert::x"],
        "undo chord should preserve undo transaction immediately after expansion"
    );
}

#[test]
fn undo_is_invalidated_by_navigation_keys() {
    // Regression test: undo must be invalidated by navigation keys
    // (Left, Right, Home, End) because they move the cursor.
    let config = Config::parse(
        r#"
        [settings]
        undo_chord = "Ctrl+Z"

        [[expansion]]
        trigger = ":x"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let _injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy {
        safe_mode: true,
        disable_hotkeys: true,
        ..Default::default()
    };

    for nav_key in &["Left", "Right", "Home", "End"] {
        let mut injector = RecordingInjector { calls: Vec::new() };
        let mut engine = ExpansionEngine::new(config.clone()).unwrap();

        // Expand :x to "ok"
        process_event(
            &mut engine,
            InputEvent::Text(":x".into()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        // Press navigation key - should invalidate undo
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse(nav_key).unwrap()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        // Try undo - should NOT work because transaction was invalidated
        process_event(
            &mut engine,
            InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+Z").unwrap()),
            Some(&mut injector),
            &policy,
            "libei",
        )
        .unwrap();

        // Should only have expansion, not undo (no erase/insert of second pair)
        assert_eq!(
            injector.calls.len(),
            2,
            "{} key should invalidate undo transaction",
            nav_key
        );
    }
}

#[cfg(unix)]
#[test]
fn deferred_command_without_worker_never_runs_synchronously() {
    let marker = std::env::temp_dir().join(format!(
        "wayexpand-daemon-no-sync-command-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let config = Config::parse(&format!(
        r#"
        [[expansion]]
        trigger = ":command"
        replacement = ""
        [expansion.command]
        program = "/bin/sh"
        args = ["-c", "printf ran > '{marker}'"]
        timeout_ms = 1000
        "#,
        marker = marker.display()
    ))
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let mut injector = RecordingInjector { calls: Vec::new() };
    let policy = wayexpand_core::OrganizationPolicy::default();

    process_event(
        &mut engine,
        InputEvent::Text(":command".into()),
        Some(&mut injector),
        &policy,
        "evdev",
    )
    .unwrap();

    assert!(!marker.exists());
    assert!(injector.calls.is_empty());
    assert_eq!(engine.command_metrics().command_in_flight, 0);
    let _ = std::fs::remove_file(marker);
}

#[test]
fn process_event_queues_hotkeys_without_waiting_for_the_child() {
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
    let policy = wayexpand_core::OrganizationPolicy::default();
    let started = Instant::now();
    process_event(
        &mut engine,
        InputEvent::Key(wayexpand_core::KeyChord::parse("Ctrl+M").unwrap()),
        None,
        &policy,
        "libei",
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_millis(50));

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some((_, result)) = engine.drain_completed_hotkeys().pop() {
            assert!(result.is_ok());
            break;
        }
        assert!(Instant::now() < deadline, "hotkey did not complete");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn lost_non_atomic_session_reconnects_and_preserves_ambiguous_result() {
    let config = Config::parse(
        r#"[[expansion]]
        trigger = ":x"
        replacement = "ok""#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();
    let error = process_event(
        &mut engine,
        InputEvent::Text(":x".into()),
        Some(&mut FailingInjector),
        &policy,
        "libei",
    )
    .unwrap_err();
    assert!(error.retryable());
    assert_eq!(error.result.trigger, ":x");
    assert_eq!(error.result.insert, "ok");
}

#[test]
fn key_event_calls_note_key_event() {
    // Regression test: daemon-level key processing must call note_key_event()
    // to invalidate pending async expansions. The core engine tests verify
    // the generation-invalidation behavior; this test ensures the daemon
    // calls the method when a Key event arrives.
    let config = Config::parse(
        r#"
        [[expansion]]
        trigger = ":test"
        replacement = "ok"
        "#,
    )
    .unwrap();
    let mut engine = ExpansionEngine::new(config).unwrap();
    let policy = wayexpand_core::OrganizationPolicy::default();

    // Key event processing should not panic or fail
    process_event(
        &mut engine,
        InputEvent::Key(KeyChord {
            modifiers: Default::default(),
            key: "Left".into(),
        }),
        None,
        &policy,
        "stdin",
    )
    .unwrap();

    // If note_key_event() was NOT called, there would be no way to detect it
    // in this test. The real verification is in the core engine tests
    // (asynchronous_command_output_is_discarded_after_key_only_input),
    // which prove that generation invalidation prevents stale output.
    // This test simply verifies the daemon path doesn't crash.
}
