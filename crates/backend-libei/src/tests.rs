use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    time::Duration,
};

struct SignalActionGuard {
    signal: libc::c_int,
    previous: libc::sigaction,
}

impl Drop for SignalActionGuard {
    fn drop(&mut self) {
        // SAFETY: `previous` was returned by sigaction for this signal before
        // the test installed its temporary handler.
        unsafe {
            libc::sigaction(self.signal, &self.previous, std::ptr::null_mut());
        }
    }
}

#[test]
fn checkpointed_sequence_stops_before_the_next_action() {
    let mut checks = 0;
    let mut sent = 0;
    let result = super::run_checkpointed(
        3,
        || {
            checks += 1;
            if checks == 2 {
                Err("focus changed")
            } else {
                Ok(())
            }
        },
        || {
            sent += 1;
            Ok(())
        },
    );

    assert_eq!(result, Err("focus changed"));
    assert_eq!(sent, 1);
}

#[test]
fn checkpointed_replacement_stops_after_a_mid_send_failure() {
    let mut sent = 0;
    let result = super::run_checkpointed(
        4,
        || Ok::<(), &'static str>(()),
        || {
            sent += 1;
            if sent == 2 {
                Err("injected output flush failure")
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(result, Err("injected output flush failure"));
    assert_eq!(sent, 2, "no replacement keys follow the failed send");
}

#[test]
fn erase_count_treats_decomposed_accent_and_devanagari_as_one() {
    for grapheme in ["e\u{301}", "ü", "u\u{308}", "👍🏽", "👩‍💻", "क्ष", "क्‍ष"]
    {
        assert_eq!(super::erase_grapheme_count(grapheme), 1, "{grapheme:?}");
    }
}

#[test]
fn backend_name_is_stable() {
    assert_eq!(super::BACKEND_NAME, "libei");
    assert_eq!(super::KEY_BACKSPACE, 14);
    assert_eq!(super::EIS_HANDSHAKE_TIMEOUT, Duration::from_secs(5));
}

#[test]
fn event_poll_has_a_bounded_deadline() {
    let (client, _server) = UnixStream::pair().unwrap();
    let context = super::ei::Context::new(client).unwrap();
    assert!(!super::events::poll_context(&context, Duration::from_millis(10)).unwrap());
}

#[test]
fn a_signal_does_not_cut_the_handshake_deadline_short() {
    static INTERRUPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    extern "C" fn count(_: libc::c_int) {
        INTERRUPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    // SAFETY: installs a handler that only bumps an atomic, without
    // SA_RESTART, so the signal interrupts poll exactly as the daemon's
    // SIGINT/SIGTERM handler does.
    let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = count as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(libc::sigaction(libc::SIGUSR2, &action, &mut previous), 0);
    }
    let previous_handler = previous.sa_sigaction;
    let previous_flags = previous.sa_flags;
    let signal_guard = SignalActionGuard {
        signal: libc::SIGUSR2,
        previous,
    };
    // SAFETY: pthread_self has no preconditions.
    let target = unsafe { libc::pthread_self() };
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signaller_done = std::sync::Arc::clone(&done);
    let signaller = std::thread::spawn(move || {
        while !signaller_done.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(20));
            // SAFETY: the test thread outlives this loop; it joins us.
            unsafe { libc::pthread_kill(target, libc::SIGUSR2) };
        }
    });
    let (client, _server) = UnixStream::pair().unwrap();
    let context = super::ei::Context::new(client).unwrap();
    let started = std::time::Instant::now();
    let result = super::events::handshake_with_timeout(&context, Duration::from_millis(300));
    let elapsed = started.elapsed();
    done.store(true, std::sync::atomic::Ordering::Release);
    signaller.join().unwrap();
    assert!(INTERRUPTS.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert!(matches!(result, Err(error) if error.to_string().contains("deadline expired")));
    assert!(
        elapsed >= Duration::from_millis(290),
        "the handshake gave up after {elapsed:?} of its 300 ms"
    );
    drop(signal_guard);

    let mut restored: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new-action pointer queries the current disposition.
    assert_eq!(
        unsafe { libc::sigaction(libc::SIGUSR2, std::ptr::null(), &mut restored) },
        0
    );
    assert_eq!(restored.sa_sigaction, previous_handler);
    // Linux may add its private SA_RESTORER bit when returning the action.
    let public_flags = libc::SA_SIGINFO
        | libc::SA_RESTART
        | libc::SA_NODEFER
        | libc::SA_RESETHAND
        | libc::SA_ONSTACK
        | libc::SA_NOCLDSTOP
        | libc::SA_NOCLDWAIT;
    assert_eq!(
        restored.sa_flags & public_flags,
        previous_flags & public_flags
    );
}

/// A minimal EIS server: completes the handshake, announces `seats`
/// seats in one burst, and keeps the connection open until `release`.
fn flooding_eis_server(
    stream: UnixStream,
    seats: usize,
    release: std::sync::mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let context = reis::eis::Context::new(stream).unwrap();
        let mut handshaker = reis::handshake::EisHandshaker::new(&context, 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let response = 'handshake: loop {
            assert!(
                std::time::Instant::now() < deadline,
                "client never finished the handshake"
            );
            let _ = context.read();
            while let Some(request) = context.pending_request() {
                let reis::PendingRequestResult::Request(request) = request else {
                    panic!("client sent an unparseable handshake request");
                };
                if let Some(response) = handshaker.handle_request(request).unwrap() {
                    break 'handshake response;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let converter = reis::request::EisRequestConverter::new(&context, response, 1);
        let _seats: Vec<_> = (0..seats)
            .map(|_| {
                converter.handle().add_seat(
                    Some("seat"),
                    reis::request::DeviceCapability::Keyboard.into(),
                )
            })
            .collect();
        while let Err(error) = context.flush() {
            assert_eq!(error, rustix::io::Errno::AGAIN, "flush failed");
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = release.recv();
    })
}

#[test]
fn a_flood_of_server_events_is_drained_in_bounded_batches_without_losing_any() {
    let (client, server) = UnixStream::pair().unwrap();
    let (release, released) = std::sync::mpsc::channel();
    let server = flooding_eis_server(server, 300, released);
    let context = super::ei::Context::new(client).unwrap();
    let (_connection, mut pump) =
        super::events::handshake_with_timeout(&context, Duration::from_secs(5)).unwrap();
    // Let the whole burst arrive, so the first drain meets a flood.
    std::thread::sleep(Duration::from_millis(200));
    let budget = super::events::DrainBudget {
        events: 100,
        time: Duration::from_secs(5),
    };
    let mut seats = 0;
    let mut batches = 0;
    loop {
        let mut batch = 0;
        let result = pump.drain(budget, |event| {
            batch += 1;
            if matches!(event, reis::event::EiEvent::SeatAdded(_)) {
                seats += 1;
            }
            Ok(())
        });
        assert!(batch <= budget.events, "one drain handled {batch} events");
        batches += 1;
        match result {
            Ok(()) => break,
            Err(super::LibeiError::EventBacklog) => {}
            Err(error) => panic!("unexpected drain failure: {error}"),
        }
        assert!(batches < 50, "the flood never finished draining");
    }
    assert!(batches > 1, "the flood should not fit one bounded drain");
    assert_eq!(seats, 300, "every event is delivered across the batches");
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn a_drain_that_runs_out_of_time_fails_closed() {
    let (client, server) = UnixStream::pair().unwrap();
    let (release, released) = std::sync::mpsc::channel();
    let server = flooding_eis_server(server, 10, released);
    let context = super::ei::Context::new(client).unwrap();
    let (_connection, mut pump) =
        super::events::handshake_with_timeout(&context, Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let budget = super::events::DrainBudget {
        events: usize::MAX,
        time: Duration::ZERO,
    };
    assert!(matches!(
        pump.drain(budget, |_| Ok(())),
        Err(super::LibeiError::EventBacklog)
    ));
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn handshake_has_a_bounded_deadline() {
    let (client, _server) = UnixStream::pair().unwrap();
    let context = super::ei::Context::new(client).unwrap();
    let error = match super::events::handshake_with_timeout(&context, Duration::from_millis(10)) {
        Err(error) => error,
        Ok(_) => panic!("idle EIS endpoint must time out"),
    };
    assert!(matches!(error, super::LibeiError::Handshake(_)));
    assert!(error.to_string().contains("deadline expired"));
}

#[test]
fn handshake_cancellation_is_observed_between_bounded_reads() {
    let (client, _server) = UnixStream::pair().unwrap();
    let context = super::ei::Context::new(client).unwrap();
    let cancellation = super::ConnectCancellation::default();
    let worker_cancellation = cancellation.clone();
    let cancel_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        worker_cancellation.cancel();
    });
    let started = std::time::Instant::now();
    let result =
        super::events::handshake_with_cancellation(&context, Duration::from_secs(5), &cancellation);
    let elapsed = started.elapsed();
    cancel_thread.join().unwrap();
    assert!(matches!(result, Err(super::LibeiError::Cancelled)));
    assert!(elapsed < Duration::from_secs(1), "cancel took {elapsed:?}");
}

#[test]
fn text_chunks_are_utf8_safe_and_protocol_sized() {
    let text = "🙂".repeat(200);
    let chunks = super::split_text_chunks(&text);
    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|chunk| {
        chunk.len() <= super::EI_TEXT_MAX_UTF8_BYTES
            && std::str::from_utf8(chunk.as_bytes()).is_ok()
    }));
    assert_eq!(chunks.concat(), text);
}

#[test]
fn empty_text_has_no_protocol_chunk() {
    assert!(super::split_text_chunks("").is_empty());
}

fn token_test_parent(name: &str) -> std::path::PathBuf {
    // The token store rejects any group/other-writable ancestor, so the
    // test base must live under a trusted chain. That depends on where
    // the tests run (a sandbox temp directory, or a Debian build tree
    // unpacked under a 0775 directory), so use the first candidate whose
    // whole chain the real validation accepts.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let candidates = [std::env::temp_dir(), workspace_dir.join("target")];
    let base = candidates
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .find(|root| super::validate_token_parent_chain(root).is_ok())
        .expect("no trusted directory chain for portal token tests")
        .join(format!(
            "wayexpand-libei-token-tests-{}",
            rustix::process::geteuid().as_raw()
        ));
    fs::create_dir_all(&base).unwrap();
    fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
    let parent = base.join(format!(
        "wayexpand-libei-token-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&parent);
    fs::create_dir_all(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    parent
}

#[test]
fn portal_token_is_readable_only_after_private_atomic_save() {
    let parent = token_test_parent("secure");
    let path = parent.join(super::PORTAL_TOKEN_FILENAME);
    super::store_portal_token_at(&path, "restoration-token").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        super::read_portal_token_at(&path).unwrap().as_deref(),
        Some("restoration-token")
    );
    assert!(super::reset_portal_token_at(&path).unwrap());
    assert!(!path.exists());
    let _ = fs::remove_dir_all(parent);
}

#[test]
fn portal_token_rejects_symlinks_and_insecure_parents() {
    let parent = token_test_parent("unsafe");
    let path = parent.join(super::PORTAL_TOKEN_FILENAME);
    let target = parent.join("target");
    fs::write(&target, "do not replace").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(super::store_portal_token_at(&path, "new-token").is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "do not replace");
    fs::remove_file(&path).unwrap();

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(super::store_portal_token_at(&path, "new-token").is_err());
    let _ = fs::remove_dir_all(parent);
}

#[test]
fn portal_token_parent_mode_security_does_not_depend_on_owner() {
    assert!(super::token_parent_mode_is_secure(0o700));
    assert!(super::token_parent_mode_is_secure(0o755));
    assert!(super::token_parent_mode_is_secure(0o1777));
    assert!(!super::token_parent_mode_is_secure(0o777));
    assert!(!super::token_parent_mode_is_secure(0o775));
}

#[test]
fn portal_token_rejects_root_owned_writable_parent_when_possible() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }

    let parent = token_test_parent("root-owned-unsafe");
    let path = parent.join(super::PORTAL_TOKEN_FILENAME);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();

    let error = super::store_portal_token_at(&path, "new-token").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

    let _ = fs::remove_dir_all(parent);
}

#[test]
fn portal_token_read_rejects_oversized_or_wrongly_permissioned_files() {
    let parent = token_test_parent("validation");
    let path = parent.join(super::PORTAL_TOKEN_FILENAME);
    fs::write(&path, "token").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(super::read_portal_token_at(&path).is_err());

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, "x".repeat(super::MAX_PORTAL_TOKEN_BYTES + 1)).unwrap();
    assert!(super::read_portal_token_at(&path).is_err());
    let _ = fs::remove_dir_all(parent);
}

#[test]
fn disabled_portal_token_persistence_neither_reads_nor_writes() {
    let parent = token_test_parent("disabled");
    let path = parent.join(super::PORTAL_TOKEN_FILENAME);
    fs::write(&path, "existing-token").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    assert_eq!(
        super::read_portal_token_if_enabled(false, Some(&path)).unwrap(),
        None,
        "disabled persistence must not read the existing token"
    );
    super::store_portal_token_if_enabled(false, Some(&path), "replacement-token").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "existing-token");

    let _ = fs::remove_dir_all(parent);
}

#[test]
fn explicit_portal_token_path_is_used_for_persistence() {
    let parent = token_test_parent("explicit-path");
    let path = parent
        .join("custom-config")
        .join(super::PORTAL_TOKEN_FILENAME);

    super::store_portal_token_if_enabled(true, Some(&path), "custom-token").unwrap();
    assert_eq!(
        super::read_portal_token_if_enabled(true, Some(&path))
            .unwrap()
            .as_deref(),
        Some("custom-token")
    );

    let _ = fs::remove_dir_all(parent);
}

#[test]
fn portal_token_survives_config_dir_writable_in_read_only_home() {
    // Verify that libei can store and retrieve tokens in a writable config
    // directory. This tests the systemd hardening scenario: ProtectHome=read-only
    // with ReadWritePaths=%h/.config/wayexpand allows token persistence after
    // restart. The security validation checks the entire parent chain; all
    // intermediate directories must be owned by the current user and not
    // writable by group/other (no 0o022 bits).
    let parent = token_test_parent("systemd-hardening");
    let config_base = parent.join(".config");
    let config_dir = config_base.join("wayexpand");
    fs::create_dir_all(&config_dir).unwrap();
    fs::set_permissions(&config_base, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();

    let token_path = config_dir.join(super::PORTAL_TOKEN_FILENAME);
    let test_token = "test-restoration-token";

    super::store_portal_token_at(&token_path, test_token).unwrap();
    assert!(token_path.exists());
    assert_eq!(
        fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Retrieve token and verify it matches
    let retrieved = super::read_portal_token_at(&token_path).unwrap();
    assert_eq!(retrieved.as_deref(), Some(test_token));

    // Clean up
    let _ = fs::remove_dir_all(parent);
}

#[test]
fn text_size_is_bounded_before_injection() {
    assert!(super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES)).is_ok());
    assert!(matches!(
        super::validate_text(&"a".repeat(super::MAX_TEXT_BYTES + 1)),
        Err(super::LibeiError::TextTooLarge { .. })
    ));
}

#[test]
fn control_characters_are_rejected_before_injection() {
    assert!(matches!(
        super::validate_text("\u{0001}"),
        Err(super::LibeiError::ControlCharacter(1))
    ));
    assert!(super::validate_text("safe\ntext\r\t").is_ok());
}

#[test]
fn transport_failures_are_retryable_but_validation_is_not() {
    assert!(super::LibeiError::Connect(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "not ready",
    ))
    .is_retryable());
    assert!(!super::LibeiError::Connect(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "not allowed",
    ))
    .is_retryable());
    assert!(super::LibeiError::Disconnected("closed".into()).is_retryable());
    assert!(super::LibeiError::Flush("closed".into()).is_retryable());
    assert!(
        super::LibeiError::Handshake(reis::Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timeout"
        ),))
        .is_retryable()
    );
    assert!(!super::LibeiError::Handshake(reis::Error::Handshake(
        reis::handshake::HandshakeError::MissingInterface,
    ))
    .is_retryable());
    assert!(!super::LibeiError::MissingRequiredDevice.is_retryable());
    assert!(!super::LibeiError::EventBacklog.is_retryable());
    assert!(!super::LibeiError::Portal("permission denied".into()).is_retryable());
    assert!(!super::LibeiError::TextTooLarge {
        length: 2,
        maximum: 1,
    }
    .is_retryable());
    assert!(!super::LibeiError::ControlCharacter(1).is_retryable());
    assert!(!super::LibeiError::Keymap("bad keymap".into()).is_retryable());
    assert!(!super::LibeiError::UnsupportedCharacter('a' as u32).is_retryable());
}

fn default_keymap() -> super::XkbKeymap {
    super::XkbKeymap::new_from_names(super::Context::new(0).unwrap(), None, 0).unwrap()
}

#[test]
fn keysym_typer_maps_shifted_and_unshifted_letters_to_the_same_key() {
    let keymap = default_keymap();
    let typer = super::KeysymTyper::build(&keymap).unwrap();
    let lower = typer.chars[&'a'];
    let upper = typer.chars[&'A'];
    assert_eq!(lower.modifiers, 0);
    assert_eq!(upper.modifiers, super::MOD_SHIFT);
    assert_eq!(
        lower.keycode, upper.keycode,
        "'a' and 'A' are the same physical key, differing only by Shift"
    );
    assert_ne!(
        Some(upper.keycode),
        typer.modifier_keycodes[0],
        "Shift itself must not be reported as a typeable character's key"
    );
}

#[test]
fn keysym_typer_does_not_claim_unreachable_characters() {
    let keymap = default_keymap();
    let typer = super::KeysymTyper::build(&keymap).unwrap();
    // No ordinary keyboard layout has a direct, unshifted/Shift-level
    // keysym for CJK ideographs -- those need an input method, which the
    // fallback deliberately cannot provide (see the module docs).
    assert!(!typer.chars.contains_key(&'中'));
}

fn layout_keymap(layout: &str) -> super::XkbKeymap {
    super::XkbKeymap::new_from_names(
        super::Context::new(0).unwrap(),
        Some(xkbcommon_rs::xkb_keymap::RuleNames {
            rules: None,
            model: None,
            layout: Some(layout.into()),
            variant: None,
            options: None,
        }),
        0,
    )
    .unwrap()
}

#[test]
fn keysym_typer_reaches_altgr_characters_on_a_german_layout() {
    let keymap = layout_keymap("de");
    let typer = super::KeysymTyper::build(&keymap).unwrap();
    let q = typer.chars[&'q'];
    let at = typer.chars[&'@'];
    assert_eq!(q.modifiers, 0);
    assert_eq!(
        at,
        super::KeyStroke {
            keycode: q.keycode,
            modifiers: super::MOD_LEVEL3,
        },
        "'@' is AltGr+Q on a German layout"
    );
    assert!(typer.chars.contains_key(&'€'));
    assert_eq!(typer.chars[&'{'].modifiers, super::MOD_LEVEL3);
    assert_eq!(typer.chars[&'ä'].modifiers, 0);
    assert_eq!(typer.chars[&'Ä'].modifiers, super::MOD_SHIFT);
    let level3 = typer.modifier_keycodes[1].expect("German layout defines AltGr");
    assert_eq!(
        typer.modifiers_for(at).collect::<Vec<_>>(),
        vec![level3],
        "only the AltGr key is held for '@'"
    );
}

#[test]
fn keysym_typer_prefers_the_simplest_chord() {
    let keymap = layout_keymap("de");
    let typer = super::KeysymTyper::build(&keymap).unwrap();
    // Digits exist unmodified; they must not be mapped to a modified
    // chord on some other key.
    assert_eq!(typer.chars[&'1'].modifiers, 0);
    assert_eq!(typer.chars[&'!'].modifiers, super::MOD_SHIFT);
}

#[test]
fn keysym_typer_refuses_to_guess_the_active_layout_of_a_multi_layout_keymap() {
    let keymap = layout_keymap("us,de");
    let mut typer = super::KeysymTyper::build(&keymap).unwrap();
    assert!(matches!(
        typer.refresh(),
        Err(super::LibeiError::UnknownActiveLayout(2))
    ));
    // A single-layout keymap has only one possible group.
    let mut single = super::KeysymTyper::build(&layout_keymap("de")).unwrap();
    assert!(single.refresh().is_ok());
}

#[test]
fn keysym_typer_follows_a_reported_layout_switch() {
    let keymap = layout_keymap("us,de");
    let mut typer = super::KeysymTyper::build(&keymap).unwrap();
    typer.note_modifiers(0, 0);
    typer.refresh().unwrap();
    let us_z = typer.chars[&'z'];
    assert!(!typer.chars.contains_key(&'ä'), "US layout has no 'ä'");

    // US -> German: 'z' moves to the key US calls 'y', '@' becomes AltGr+Q.
    typer.note_modifiers(1, 0);
    typer.refresh().unwrap();
    let de_z = typer.chars[&'z'];
    assert_ne!(us_z.keycode, de_z.keycode, "QWERTZ swaps z and y");
    assert_eq!(typer.chars[&'y'].keycode, us_z.keycode);
    assert_eq!(typer.chars[&'@'].modifiers, super::MOD_LEVEL3);
    assert_eq!(typer.chars[&'ä'].modifiers, 0);

    // And back again.
    typer.note_modifiers(0, 0);
    typer.refresh().unwrap();
    assert_eq!(typer.chars[&'z'], us_z);
}

#[test]
fn keysym_typer_accounts_for_caps_lock() {
    let keymap = layout_keymap("us");
    let lock = 1
        << keymap
            .mod_get_index("Lock")
            .expect("keymap has a Lock modifier");
    let mut typer = super::KeysymTyper::build(&keymap).unwrap();
    typer.note_modifiers(0, lock);
    typer.refresh().unwrap();
    // With Caps Lock on, the unmodified letter key types uppercase.
    assert_eq!(typer.chars[&'A'].modifiers, 0);
    assert_eq!(typer.chars[&'a'].modifiers, super::MOD_SHIFT);
    assert_eq!(typer.chars[&'1'].modifiers, 0);
}

#[test]
fn keysym_typer_rejects_a_layout_group_the_keymap_does_not_have() {
    let keymap = layout_keymap("us");
    let mut typer = super::KeysymTyper::build(&keymap).unwrap();
    typer.note_modifiers(3, 0);
    assert!(matches!(typer.refresh(), Err(super::LibeiError::Keymap(_))));
}

#[test]
fn unavailable_device_errors_are_retryable_and_unknown_layout_is_not() {
    assert!(super::LibeiError::DeviceUnavailable("removed by the server").is_retryable());
    assert!(!super::LibeiError::UnknownActiveLayout(2).is_retryable());
}
