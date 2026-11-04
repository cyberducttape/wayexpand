use super::*;
use std::io::Read;
use std::sync::atomic::Ordering;

#[test]
fn simultaneous_explain_clients_receive_their_own_fifo_answers() {
    let (queue, requests) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
    let first_queue = queue.clone();
    let first = thread::spawn(move || {
        explain_request("explain-json .first", &first_queue)
            .unwrap()
            .unwrap()
    });

    let second_queue = queue.clone();
    let second = thread::spawn(move || {
        explain_request("explain-json .second", &second_queue)
            .unwrap()
            .unwrap()
    });

    for _ in 0..2 {
        let request = requests.recv_timeout(Duration::from_millis(500)).unwrap();
        request
            .reply
            .try_send(format!("answer for {}", request.text))
            .unwrap();
    }
    assert_eq!(first.join().unwrap(), "answer for .first");
    assert_eq!(second.join().unwrap(), "answer for .second");
}

#[test]
fn saturated_explanation_queue_returns_retryable_response_without_overwriting() {
    let (queue, _requests) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
    let mut held_replies = Vec::new();
    for index in 0..EXPLAIN_QUEUE_CAPACITY {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        held_replies.push(response);
        queue
            .try_send(ExplainRequest {
                text: format!(".pending{index}"),
                json: true,
                reply,
            })
            .unwrap();
    }

    let json = explain_request("explain-json .extra", &queue)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({"error": "busy", "retryable": true})
    );
    let text = explain_request("explain .extra", &queue).unwrap().unwrap();
    assert_eq!(text, "explain unavailable: daemon is busy; retry\n");
    drop(held_replies);
}

#[test]
fn diagnostics_worker_answers_clipboard_snippets_without_rendering_them() {
    let config = wayexpand_core::Config::parse(
        "[settings]\nallow_clipboard = true\n\
             [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
    )
    .unwrap();
    let validated = Arc::new(config.validate_snapshot().unwrap());
    let snapshot = Arc::new(Mutex::new(Some(ExplainSnapshot {
        validated,
        backend: "test",
        capture: "test-capture",
        output: "none",
        connection_detail: "test mode",
        output_connected: true,
        capabilities: None,
        paused: false,
        requested_pause: false,
        profile_switch_pending: false,
        sensitive: false,
        composition: false,
        form_open: false,
        window: None,
        form_guarantee: wayexpand_core::ReplacementGuarantee::Unsupported,
        commands_disabled: false,
        direct_commands_disabled: false,
        title_matching_disabled: false,
    })));
    for detailed in [true, false] {
        let (sender, receiver) = std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY);
        let worker_snapshot = Arc::clone(&snapshot);
        let worker = thread::spawn(move || {
            explain_worker(
                receiver,
                worker_snapshot,
                detailed,
                Arc::new(AtomicBool::new(false)),
            )
        });

        let started = Instant::now();
        let answer = explain_request("explain-json :clip", &sender)
            .unwrap()
            .unwrap();
        let elapsed = started.elapsed();
        let response: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(response["matched"], true);
        assert_eq!(response["would_expand"], true);
        assert_eq!(response["suppression_reason"], "none");
        if detailed {
            assert_eq!(response["snippet"], ":clip");
            assert!(response["checks"].as_array().unwrap().iter().any(|check| {
                check["name"] == "replacement"
                    && check["detail"] == "not rendered by control-socket diagnostics"
            }));
        } else {
            assert_eq!(
                response,
                serde_json::json!({
                    "matched": true,
                    "would_expand": true,
                    "suppression_reason": "none",
                })
            );
        }
        assert!(elapsed < Duration::from_millis(500));

        drop(sender);
        worker.join().unwrap();
    }
}

#[test]
fn restricted_diagnostics_redact_snippet_and_dynamic_value_metadata() {
    let explanation = wayexpand_core::Explanation {
        typed: "::payroll-secret".to_owned(),
        snippet: Some("::payroll-secret".to_owned()),
        checks: vec![
            wayexpand_core::ExplainCheck {
                name: "command",
                status: wayexpand_core::CheckStatus::Info,
                detail: "runs /usr/local/libexec/private-helper".to_owned(),
            },
            wayexpand_core::ExplainCheck {
                name: "variables",
                status: wayexpand_core::CheckStatus::Info,
                detail: "reads env:PAYROLL_TOKEN and clipboard".to_owned(),
            },
            wayexpand_core::ExplainCheck {
                name: "replacement",
                status: wayexpand_core::CheckStatus::Pass,
                detail: "renders to 47 characters; Unicode U+202F at character 12".to_owned(),
            },
        ],
    };

    let response = restricted_explain_response(&explanation, true);
    let value: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "matched": true,
            "would_expand": true,
            "suppression_reason": "none",
        })
    );
    for private in [
        "::payroll-secret",
        "/usr/local/libexec/private-helper",
        "PAYROLL_TOKEN",
        "clipboard",
        "47",
        "U+202F",
        "character 12",
    ] {
        assert!(
            !response.contains(private),
            "leaked {private:?}: {response}"
        );
    }
    assert_eq!(
        restricted_explain_response(&explanation, false),
        "Snippet match: yes\nWould expand: yes\nReason: none\n"
    );
}

#[test]
fn socket_diagnostic_flood_does_not_stall_keyboard_processing_with_slow_clipboard() {
    let directory = std::env::temp_dir().join(format!(
        "wayexpand-explain-stress-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = directory.join("control.sock");
    let server = ControlServer::start_at(&socket, false, false).unwrap();
    let config = wayexpand_core::Config::parse(
        "[settings]\nallow_clipboard = true\n\
             [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
    )
    .unwrap();
    server.set_explain_snapshot(ExplainSnapshot {
        validated: Arc::new(config.validate_snapshot().unwrap()),
        backend: "test",
        capture: "test-capture",
        output: "none",
        connection_detail: "test mode",
        output_connected: true,
        capabilities: None,
        paused: false,
        requested_pause: false,
        profile_switch_pending: false,
        sensitive: false,
        composition: false,
        form_open: false,
        window: None,
        form_guarantee: wayexpand_core::ReplacementGuarantee::Unsupported,
        commands_disabled: false,
        direct_commands_disabled: false,
        title_matching_disabled: false,
    });

    let stop_input = Arc::new(AtomicBool::new(false));
    let max_input_delay_ns = Arc::new(AtomicU64::new(0));
    let clipboard_reads = Arc::new(AtomicU64::new(0));
    let input_stop = Arc::clone(&stop_input);
    let max_delay = Arc::clone(&max_input_delay_ns);
    let reads = Arc::clone(&clipboard_reads);
    let input_thread = thread::spawn(move || {
        let mut engine = wayexpand_core::ExpansionEngine::new(
            wayexpand_core::Config::parse(
                "[settings]\nallow_clipboard = true\n\
                     [[expansion]]\ntrigger = ':clip'\nreplacement = '{{clipboard}}'\n",
            )
            .unwrap(),
        )
        .unwrap();
        engine.set_clipboard_reader(Some(wayexpand_core::ClipboardReader(Arc::new(move || {
            reads.fetch_add(1, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(250));
            Some("slow clipboard".to_owned())
        }))));
        let mut pending = Vec::new();
        while !input_stop.load(Ordering::Acquire) {
            let started = Instant::now();
            engine.process_deferred_char_into('x', &mut pending);
            max_delay.fetch_max(
                started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
        }
    });

    let clients = (0..CONTROL_WORKERS)
        .map(|_| {
            let socket = socket.clone();
            thread::spawn(move || {
                for _ in 0..12 {
                    let mut stream = UnixStream::connect(&socket).unwrap();
                    stream.write_all(b"explain-json :clip\n").unwrap();
                    let mut response = String::new();
                    stream.read_to_string(&mut response).unwrap();
                    assert!(!response.is_empty());
                }
            })
        })
        .collect::<Vec<_>>();
    for client in clients {
        client.join().unwrap();
    }
    stop_input.store(true, Ordering::Release);
    input_thread.join().unwrap();

    assert_eq!(clipboard_reads.load(Ordering::Relaxed), 0);
    assert!(
        max_input_delay_ns.load(Ordering::Relaxed) < 100_000_000,
        "maximum simulated input processing gap was {} ms",
        max_input_delay_ns.load(Ordering::Relaxed) / 1_000_000
    );
    drop(server);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn focus_read_does_not_authorize_insert_target_in_restricted_mode() {
    let insert = Arc::new(Mutex::new(None));
    let focus = Arc::new(Mutex::new(FocusSnapshot {
        generation: 42,
        token: Some("deadbeef".into()),
        exact_window_identity: true,
    }));
    let flags = Flags {
        reload: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
        insert: Arc::clone(&insert),
        require_verified_insert: true,
        disable_control_socket_mutations: true,
        focus,
        explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
        usage_clear: Arc::new(Mutex::new(None)),
        status: Arc::new(Mutex::new("running".into())),
        waker: Arc::default(),
    };
    let send = |command: &str| {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(command.as_bytes()).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handle_request(server, flags.clone()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    };

    assert_eq!(
            send("focus\n"),
            "focus_generation=\nfocus_token=\nfocus_identity=exact\natomic_replace=false\nverified_replace=false\n"
        );
    for request in [
        "insert-target 42 deadbeef ;sig\n",
        "insert ;sig\n",
        "reload\n",
        "pause\n",
        "resume\n",
        "stop\n",
        "stats-clear /tmp/usage.json\n",
    ] {
        assert_eq!(
            send(request).as_bytes(),
            CONTROL_MUTATIONS_DISABLED_RESPONSE,
            "request {request:?}"
        );
    }
    assert!(insert.lock().unwrap().is_none());
    assert!(!flags.stop.load(Ordering::Acquire));
    assert!(!flags.reload.load(Ordering::Acquire));
    assert!(!flags.pause.load(Ordering::Acquire));
    assert_eq!(send("status\n"), "running\nrunning\n");
}

#[test]
fn verified_target_policy_keeps_focus_identifiers_for_verified_inserts() {
    let insert = Arc::new(Mutex::new(None));
    let flags = Flags {
        reload: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
        insert: Arc::clone(&insert),
        require_verified_insert: true,
        disable_control_socket_mutations: false,
        focus: Arc::new(Mutex::new(FocusSnapshot {
            generation: 9,
            token: Some("deadbeef".into()),
            exact_window_identity: true,
        })),
        explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
        usage_clear: Arc::new(Mutex::new(None)),
        status: Arc::new(Mutex::new(
            "running\ninject_atomic_replace=true\nreplacement_assurance=verified\n".into(),
        )),
        waker: Arc::default(),
    };
    let send = |command: &str| {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(command.as_bytes()).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handle_request(server, flags.clone()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    };

    // The picker and `insert --verified` read the token, then insert.
    assert_eq!(
            send("focus\n"),
            "focus_generation=9\nfocus_token=deadbeef\nfocus_identity=exact\natomic_replace=true\nverified_replace=true\n"
        );
    assert_eq!(
        send("insert-target 9 deadbeef ;sig\n"),
        "insert scheduled\n"
    );
    assert!(insert.lock().unwrap().is_some());
    assert_eq!(send("insert ;sig\n"), UNVERIFIED_INSERT_REFUSED);
}

#[test]
fn control_mutation_classifier_covers_all_state_changing_commands() {
    for request in [
        "insert ;sig",
        "insert-target 42 deadbeef ;sig",
        "reload",
        "stop",
        "pause",
        "resume",
        "stats-clear /tmp/usage.json",
    ] {
        assert!(is_mutating_request(request), "{request}");
    }
    for request in ["status", "focus", "explain ;sig", "explain-json ;sig"] {
        assert!(!is_mutating_request(request), "{request}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn control_peer_credentials_match_the_daemon_uid() {
    let (stream, _peer) = UnixStream::pair().unwrap();
    assert_eq!(
        peer_uid(&stream).unwrap(),
        rustix::process::geteuid().as_raw()
    );
    let uid = rustix::process::geteuid().as_raw();
    assert!(peer_uid_is_authorized(uid, uid));
    assert!(!peer_uid_is_authorized(uid.saturating_add(1), uid));
    let expected = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    assert!(validate_peer_credential_length(expected).is_ok());
    let error = validate_peer_credential_length(expected.saturating_sub(1)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn shutdown_completes_when_the_socket_path_was_removed() {
    let directory = std::env::temp_dir().join(format!(
        "wayexpand-control-removed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("wayexpand.sock");
    let server = ControlServer::start_at(&path, false, false).unwrap();
    // For example, a session cleanup removed the runtime directory.
    fs::remove_file(&path).unwrap();

    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(server);
        let _ = done.send(());
    });
    assert!(
        finished.recv_timeout(Duration::from_secs(5)).is_ok(),
        "control server shutdown hung after its socket path was removed"
    );
    let _ = fs::remove_dir_all(directory);
}

fn request(
    command: &str,
    reload: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
    status: &Arc<Mutex<String>>,
) -> String {
    let (mut client, server) = UnixStream::pair().unwrap();
    let reload_worker = Arc::clone(reload);
    let stop_worker = Arc::clone(stop);
    let pause_worker = Arc::clone(pause);
    let status_worker = Arc::clone(status);
    let join = thread::spawn(move || {
        let insert = Arc::new(Mutex::new(None));
        let focus = Arc::new(Mutex::new(FocusSnapshot::default()));
        handle_request(
            server,
            Flags {
                reload: reload_worker,
                stop: stop_worker,
                pause: pause_worker,
                insert,
                require_verified_insert: false,
                disable_control_socket_mutations: false,
                focus,
                explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
                usage_clear: Arc::new(Mutex::new(None)),
                status: status_worker,
                waker: Arc::default(),
            },
        )
        .unwrap();
    });
    client.write_all(command.as_bytes()).unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    join.join().unwrap();
    response
}

#[test]
fn lifecycle_commands_set_flags_and_reply() {
    let reload = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new("source=stdin\nbackend=none".into()));
    assert_eq!(
        request("status\n", &reload, &stop, &pause, &status),
        "running\nsource=stdin\nbackend=none\n"
    );
    assert_eq!(
        request("reload\n", &reload, &stop, &pause, &status),
        "reload scheduled\n"
    );
    assert!(reload.load(Ordering::Acquire));
    assert_eq!(
        request("pause\n", &reload, &stop, &pause, &status),
        "paused\n"
    );
    assert!(pause.load(Ordering::Acquire));
    assert_eq!(
        request("resume\n", &reload, &stop, &pause, &status),
        "resumed\n"
    );
    assert!(!pause.load(Ordering::Acquire));
    assert_eq!(
        request("stop\n", &reload, &stop, &pause, &status),
        "stopping\n"
    );
    assert!(stop.load(Ordering::Acquire));
}

#[test]
fn verified_insert_policy_refuses_requests_without_a_focus_target() {
    let insert = Arc::new(Mutex::new(None));
    let flags = Flags {
        reload: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
        insert: Arc::clone(&insert),
        require_verified_insert: true,
        disable_control_socket_mutations: false,
        focus: Arc::new(Mutex::new(FocusSnapshot::default())),
        explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
        usage_clear: Arc::new(Mutex::new(None)),
        status: Arc::new(Mutex::new(String::new())),
        waker: Arc::default(),
    };
    let send = |command: &str| {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(command.as_bytes()).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handle_request(server, flags.clone()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    };

    assert_eq!(send("insert ;sig\n"), UNVERIFIED_INSERT_REFUSED);
    assert!(insert.lock().unwrap().is_none(), "nothing may be queued");

    assert_eq!(send("insert-target 4 abcd ;sig\n"), "insert scheduled\n");
    let queued = insert.lock().unwrap().clone().unwrap();
    assert_eq!(queued.focus_token.as_deref(), Some("abcd"));
    assert_eq!(queued.focus_generation, Some(4));
}

#[test]
fn insert_takes_the_trigger_verbatim_and_refuses_control_characters() {
    let reload = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let insert = Arc::new(Mutex::new(None));
    let focus = Arc::new(Mutex::new(FocusSnapshot::default()));
    let status = Arc::new(Mutex::new(String::new()));
    let flags = Flags {
        reload,
        stop,
        pause,
        insert: Arc::clone(&insert),
        require_verified_insert: false,
        disable_control_socket_mutations: false,
        focus: Arc::clone(&focus),
        explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
        usage_clear: Arc::new(Mutex::new(None)),
        status,
        waker: Arc::default(),
    };
    let send = |command: &str| {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(command.as_bytes()).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handle_request(server, flags.clone()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    };
    for (line, trigger) in [
        ("insert ;sig\n", ";sig"),
        ("insert :sig \r\n", ":sig "),
        ("insert  lead\n", " lead"),
        ("insert 🙂x\n", "🙂x"),
    ] {
        assert_eq!(send(line), "insert scheduled\n", "{line:?}");
        assert_eq!(
            insert
                .lock()
                .unwrap()
                .as_ref()
                .map(|request| request.trigger.as_str()),
            Some(trigger)
        );
    }
    *focus.lock().unwrap() = FocusSnapshot {
        generation: 4,
        token: Some("abcd".into()),
        exact_window_identity: true,
    };
    assert_eq!(
            send("focus\n"),
            "focus_generation=4\nfocus_token=abcd\nfocus_identity=exact\natomic_replace=false\nverified_replace=false\n"
        );
    assert_eq!(send("insert-target 4 abcd ;target\n"), "insert scheduled\n");
    assert_eq!(
        insert
            .lock()
            .unwrap()
            .as_ref()
            .map(|request| request.focus_token.as_deref()),
        Some(Some("abcd"))
    );
    let too_long = format!("insert {}\n", "x".repeat(129));
    for bad in ["insert\n", "insert \n", "insert a\tb\n", too_long.as_str()] {
        *insert.lock().unwrap() = None;
        assert_eq!(send(bad), "invalid trigger\n", "{bad:?}");
        assert!(insert.lock().unwrap().is_none());
    }
}

#[test]
fn maximum_exact_focus_insert_fits_control_frame() {
    let window = crate::WindowContext {
        app_id: None,
        title: None,
        instance_id: Some("x".repeat(192)),
    };
    let token = crate::focus::focus_token(&window).unwrap();
    let trigger = "🙂".repeat(128);
    let command = format!("insert-target {} {} {}\n", u64::MAX, token, trigger);

    assert!(command.len() <= MAX_COMMAND_BYTES);
    assert!(insert_request(&command).unwrap().is_some());
}

#[test]
fn only_an_unusable_socket_stops_the_listener() {
    let error = |errno: rustix::io::Errno| std::io::Error::from_raw_os_error(errno.raw_os_error());
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::CONNABORTED)),
        AcceptFailure::Retry
    );
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::INTR)),
        AcceptFailure::Retry
    );
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::MFILE)),
        AcceptFailure::Backoff
    );
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::NFILE)),
        AcceptFailure::Backoff
    );
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::INVAL)),
        AcceptFailure::Fatal
    );
    assert_eq!(
        accept_failure(&error(rustix::io::Errno::BADF)),
        AcceptFailure::Fatal
    );
}

#[test]
fn unknown_command_is_nonfatal() {
    let reload = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new("starting".into()));
    assert_eq!(
            request("bogus\n", &reload, &stop, &pause, &status),
            "unknown command; expected status, focus, reload, pause, resume, stop, insert <trigger>, or explain <text>\n"
        );
    assert!(!reload.load(Ordering::Acquire));
    assert!(!stop.load(Ordering::Acquire));
}

#[test]
fn oversized_command_is_bounded_and_nonfatal() {
    let reload = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new("starting".into()));
    let command = format!("{}\n", "x".repeat(MAX_COMMAND_BYTES + 1024));
    assert_eq!(
            request(&command, &reload, &stop, &pause, &status),
        "unknown command; expected status, focus, reload, pause, resume, stop, insert <trigger>, or explain <text>\n"
        );
    assert!(!reload.load(Ordering::Acquire));
    assert!(!stop.load(Ordering::Acquire));
}

#[test]
fn a_request_split_across_writes_is_read_whole() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let writer = std::thread::spawn(move || {
        client.write_all(b"sta").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        client.write_all(b"tus\n").unwrap();
        client
    });
    let command = read_command(&mut server, Duration::from_secs(5)).unwrap();
    assert_eq!(command.as_deref(), Some("status\n"));
    drop(writer.join().unwrap());
}

#[test]
fn an_oversized_request_is_refused_without_waiting_for_its_end() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    // No newline, and the client stays connected.
    client.write_all(&[b'x'; MAX_COMMAND_BYTES + 1]).unwrap();
    let started = Instant::now();
    assert_eq!(
        read_command(&mut server, Duration::from_secs(5)).unwrap(),
        None
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(client);
}

#[test]
fn a_trickling_client_cannot_outlast_the_request_deadline() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let trickle_stop = Arc::clone(&stop);
    // One byte every 20 ms never trips a per-read timeout.
    let trickler = std::thread::spawn(move || {
        while !trickle_stop.load(Ordering::Acquire) {
            if client.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let started = Instant::now();
    let error = read_command(&mut server, Duration::from_millis(300)).unwrap_err();
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Release);
    drop(server);
    trickler.join().unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(
        elapsed < Duration::from_secs(1),
        "the request held its worker for {elapsed:?}"
    );
}

#[test]
fn overloaded_control_request_gets_machine_readable_busy_response() {
    let rejected = AtomicU64::new(0);
    let (mut client, server) = UnixStream::pair().unwrap();
    // An unread, oversized request must not turn the close into a
    // connection reset.
    client
        .write_all(&vec![b'x'; MAX_COMMAND_BYTES * 4])
        .unwrap();
    reject_busy(server, &rejected);
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert_eq!(response, "error=busy\nretryable=true\n");
    assert_eq!(rejected.load(Ordering::Acquire), 1);
}

#[test]
fn busy_response_keeps_socket_open_for_request_bytes_sent_after_connect() {
    let (mut client, server) = UnixStream::pair().unwrap();
    let server_thread = thread::spawn(move || {
        let rejected = AtomicU64::new(0);
        reject_busy(server, &rejected);
        rejected.load(Ordering::Acquire)
    });

    // Read the response before sending the request to force the accept-
    // then-write ordering that used to close the socket with a broken
    // pipe. The server's write half is shut down, but its read half must
    // stay alive long enough to consume this bounded request.
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert_eq!(response, "error=busy\nretryable=true\n");
    client.write_all(b"explain-json :clip\n").unwrap();
    assert_eq!(server_thread.join().unwrap(), 1);
}

#[test]
fn stale_socket_policy_requires_socket_and_matching_owner() {
    let path = std::env::temp_dir().join(format!("wayexpand-control-test-{}", std::process::id()));
    let listener = UnixListener::bind(&path).unwrap();
    let metadata = fs::symlink_metadata(&path).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    assert!(is_owned_socket(&metadata, uid));
    let identity = (metadata.dev(), metadata.ino());
    assert!(is_original_socket(&metadata, identity, uid));
    assert!(!is_original_socket(&metadata, (0, 0), uid));
    assert!(!is_owned_socket(&metadata, uid.saturating_add(1)));
    drop(listener);
    fs::remove_file(path).unwrap();

    let regular =
        std::env::temp_dir().join(format!("wayexpand-control-regular-{}", std::process::id()));
    fs::write(&regular, b"not a socket").unwrap();
    let metadata = fs::metadata(&regular).unwrap();
    assert!(!is_owned_socket(&metadata, uid));
    fs::remove_file(regular).unwrap();
}

#[test]
fn only_refused_or_missing_connections_are_stale_socket_errors() {
    assert!(is_stale_socket_error(&std::io::Error::from(
        std::io::ErrorKind::ConnectionRefused,
    )));
    assert!(is_stale_socket_error(&std::io::Error::from(
        std::io::ErrorKind::NotFound,
    )));
    assert!(!is_stale_socket_error(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied,
    )));
    assert!(!is_stale_socket_error(&std::io::Error::from(
        std::io::ErrorKind::Other,
    )));
}

#[test]
fn socket_parent_must_be_private_and_user_owned() {
    let parent =
        std::env::temp_dir().join(format!("wayexpand-control-parent-{}", std::process::id()));
    fs::create_dir(&parent).unwrap();
    // Explicit mode rather than the ambient umask: a default of 002
    // (Debian/Ubuntu user-private-group setups) creates this 0775, which
    // this check correctly rejects as group-writable.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = parent.join("wayexpand.sock");
    assert!(validate_socket_parent(&socket).is_ok());

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(validate_socket_parent(&socket).is_err());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir(parent).unwrap();
}

#[test]
fn root_owned_world_writable_mode_is_rejected() {
    // Keep this regression test independent of test-runner privileges:
    // the validation rule is about mode bits, regardless of uid.
    assert!(!socket_parent_mode_is_secure(0o0777));
    assert!(socket_parent_mode_is_secure(0o1777));
    assert!(socket_parent_mode_is_secure(0o0755));
}

#[test]
fn socket_ancestors_must_be_trusted() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let root =
        std::env::temp_dir().join(format!("wayexpand-control-ancestor-{}", std::process::id()));
    let untrusted = root.join("untrusted");
    let inner = untrusted.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&untrusted, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o700)).unwrap();
    rustix::fs::chown(&untrusted, Some(rustix::fs::Uid::from_raw(65_534)), None).unwrap();

    assert!(validate_socket_parent(&inner.join("wayexpand.sock")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn socket_parent_symlink_is_resolved_before_binding() {
    let root =
        std::env::temp_dir().join(format!("wayexpand-control-symlink-{}", std::process::id()));
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir_all(&target).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert_eq!(
        secure_socket_path(&link.join("wayexpand.sock")).unwrap(),
        target.join("wayexpand.sock")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn state_changing_requests_wake_the_reactor() {
    let waker = crate::waker::Waker::new().unwrap();
    let slot: crate::waker::WakerSlot = Arc::default();
    slot.set(waker.clone()).ok().unwrap();
    let flags = Flags {
        reload: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
        insert: Arc::new(Mutex::new(None)),
        require_verified_insert: false,
        disable_control_socket_mutations: false,
        focus: Arc::new(Mutex::new(FocusSnapshot::default())),
        explain: std::sync::mpsc::sync_channel(EXPLAIN_QUEUE_CAPACITY).0,
        usage_clear: Arc::new(Mutex::new(None)),
        status: Arc::new(Mutex::new(String::new())),
        waker: slot,
    };
    let fd = waker.fd();
    let woken = || {
        let mut fds = [rustix::event::PollFd::new(
            &*fd,
            rustix::event::PollFlags::IN,
        )];
        let zero = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let ready = rustix::event::poll(&mut fds, Some(&zero)).unwrap() > 0;
        if ready {
            let mut buffer = [0_u8; 8];
            rustix::io::read(&*fd, &mut buffer).unwrap();
        }
        ready
    };
    for (command, wakes) in [
        ("status\n", false),
        ("focus\n", false),
        ("pause\n", true),
        ("resume\n", true),
        ("reload\n", true),
        ("insert :x\n", true),
        ("stop\n", true),
    ] {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(command.as_bytes()).unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        handle_request(server, flags.clone()).unwrap();
        assert_eq!(woken(), wakes, "{command:?}");
    }
}
