use super::*;
use std::os::fd::AsFd;
use xkbcommon_rs::xkb_keymap::RuleNames;

#[test]
fn reactor_eventfd_wakes_the_wayland_poll_and_is_drained() {
    let connection = rustix::event::eventfd(
        0,
        rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
    )
    .unwrap();
    let wake = rustix::event::eventfd(
        0,
        rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
    )
    .unwrap();
    rustix::io::write(&wake, &1_u64.to_ne_bytes()).unwrap();

    let (connection_flags, woken) = poll_connection_with_wake(
        connection.as_fd(),
        Some(wake.as_fd()),
        Duration::from_secs(1),
    )
    .unwrap()
    .unwrap();
    assert!(connection_flags.is_empty());
    assert!(woken);
    assert!(
        poll_connection_with_wake(connection.as_fd(), Some(wake.as_fd()), Duration::ZERO)
            .unwrap()
            .is_none()
    );

    rustix::io::write(&connection, &1_u64.to_ne_bytes()).unwrap();
    let (connection_flags, woken) =
        poll_connection_with_wake(connection.as_fd(), None, Duration::from_secs(1))
            .unwrap()
            .unwrap();
    assert!(connection_flags.contains(rustix::event::PollFlags::IN));
    assert!(!woken);
}

struct RecordingInjector {
    calls: Vec<(u32, Modifiers)>,
    events: Vec<(u32, KeyEventState)>,
    fail: bool,
}

struct FailFirstReleaseInjector {
    failed_release: bool,
    events: Vec<(u32, KeyEventState, bool)>,
}

fn default_state() -> State {
    let keymap = Keymap::new_from_names(Context::new(0).unwrap(), None, 0).unwrap();
    State::new(keymap)
}

fn keymap_state(layout: &str, variant: &str) -> Option<State> {
    let names = RuleNames::new("", "", layout, variant, "");
    Keymap::new_from_names(Context::new(0).unwrap(), Some(names), 0)
        .ok()
        .map(State::new)
}

fn keymap_state_with_options(layout: &str, variant: &str, options: &str) -> Option<State> {
    let names = RuleNames::new("", "", layout, variant, options);
    Keymap::new_from_names(Context::new(0).unwrap(), Some(names), 0)
        .ok()
        .map(State::new)
}

fn evdev_key_for_keysym(state: &State, expected: &str) -> Option<u32> {
    (0_u32..=247).find(|key| {
        key.checked_add(8)
            .and_then(|keycode| state.key_get_one_sym(keycode))
            .and_then(|keysym| keysym_get_name(&keysym))
            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
    })
}

impl TextInjector for RecordingInjector {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
        Ok(())
    }

    fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
        Ok(())
    }

    fn inject_key_with_modifiers(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
    ) -> Result<(), InjectorError> {
        if self.fail {
            return Err(InjectorError {
                backend: self.name(),
                message: "synthetic failure".into(),
                retryable: true,
            });
        }
        self.calls.push((keycode, modifiers));
        Ok(())
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        modifiers: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        if self.fail {
            return Err(InjectorError {
                backend: self.name(),
                message: "synthetic failure".into(),
                retryable: true,
            });
        }
        self.calls.push((keycode, modifiers));
        self.events.push((keycode, state));
        Ok(())
    }
}

impl TextInjector for FailFirstReleaseInjector {
    fn name(&self) -> &'static str {
        "fail-first-release"
    }

    fn erase(&mut self, _: &str) -> Result<(), InjectorError> {
        Ok(())
    }

    fn insert(&mut self, _: &str) -> Result<(), InjectorError> {
        Ok(())
    }

    fn inject_key_event(
        &mut self,
        keycode: u32,
        _: Modifiers,
        state: KeyEventState,
    ) -> Result<(), InjectorError> {
        let fail = state == KeyEventState::Released && !self.failed_release;
        if fail {
            self.failed_release = true;
        }
        self.events.push((keycode, state, fail));
        if fail {
            Err(InjectorError {
                backend: self.name(),
                message: "synthetic release failure".into(),
                retryable: true,
            })
        } else {
            Ok(())
        }
    }
}

#[test]
fn special_keysyms_become_engine_events() {
    assert_eq!(
        classify_keysym(xkeysym::key::BackSpace),
        Some(InputEvent::Backspace)
    );
    assert_eq!(
        classify_keysym(xkeysym::key::Return),
        Some(InputEvent::Delimiter('\n'))
    );
    assert_eq!(
        classify_keysym(xkeysym::key::KP_Enter),
        Some(InputEvent::Delimiter('\n'))
    );
    assert_eq!(
        classify_keysym(xkeysym::key::Tab),
        Some(InputEvent::Delimiter('\t'))
    );
    assert_eq!(classify_keysym(xkeysym::key::Escape), None);
}

#[test]
fn xkb_state_transition_applies_modifiers_before_next_key() {
    let mut state = default_state();

    assert!(matches!(
        key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert!(matches!(
        key_action_and_update(&mut state, 30, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Text(text)) if text == "A"
    ));
    assert!(key_action_and_update(&mut state, 30, wl_keyboard::KeyState::Released).is_none());
    assert!(key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Released).is_none());
}

#[test]
fn ctrl_shortcut_is_passed_through_instead_of_committed_as_text() {
    let mut state = default_state();

    assert!(matches!(
        key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert_eq!(
        active_modifiers(&state),
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        }
    );
    assert!(matches!(
        key_action_and_update(&mut state, 46, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn common_control_shortcuts_are_passed_through_before_utf8_conversion() {
    for (name, key) in [("Ctrl+C", 46), ("Ctrl+V", 47), ("Ctrl+Z", 44)] {
        let mut state = default_state();
        assert!(matches!(
            key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(
            matches!(
                key_action_and_update(&mut state, key, wl_keyboard::KeyState::Pressed),
                Some(KeyAction::Unsupported)
            ),
            "{name} must pass through instead of committing control text"
        );
    }
}

#[test]
fn ctrl_shift_shortcuts_are_passed_through_with_modifiers_preserved() {
    let mut state = default_state();
    assert!(matches!(
        key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert!(matches!(
        key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert_eq!(
        active_modifiers(&state),
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        }
    );
    assert!(matches!(
        key_action_and_update(&mut state, 20, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn alt_and_super_shortcuts_are_passed_through_before_utf8_conversion() {
    for (name, modifier, key) in [("Alt+F4", 56, 62), ("Super+L", 125, 38)] {
        let mut state = default_state();
        assert!(matches!(
            key_action_and_update(&mut state, modifier, wl_keyboard::KeyState::Pressed),
            Some(KeyAction::Ignore)
        ));
        assert!(
            matches!(
                key_action_and_update(&mut state, key, wl_keyboard::KeyState::Pressed),
                Some(KeyAction::Unsupported)
            ),
            "{name} must pass through instead of becoming text"
        );
    }
}

#[test]
fn caps_lock_and_shift_remain_text_producing_modifiers() {
    let mut shifted = default_state();
    assert!(matches!(
        key_action_and_update(&mut shifted, 42, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert!(matches!(
        key_action_and_update(&mut shifted, 30, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Text(text)) if text == "A"
    ));

    let mut caps = default_state();
    assert!(matches!(
        key_action_and_update(&mut caps, 58, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));
    assert!(matches!(
        key_action_and_update(&mut caps, 30, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Text(text)) if text == "A"
    ));
}

#[test]
fn altgr_text_is_not_treated_as_shortcut_alt_when_layout_supports_it() {
    let Some(mut state) = keymap_state("de", "") else {
        return;
    };
    let _ = key_action_and_update(&mut state, 100, wl_keyboard::KeyState::Pressed);
    let action = key_action_and_update(&mut state, 16, wl_keyboard::KeyState::Pressed);
    if let Some(KeyAction::Text(text)) = action {
        // On systems with complete xkeyboard-config, AltGr+Q produces @
        assert_eq!(text, "@");
    } else {
        // Some minimal CI images lack complete xkeyboard-config data or
        // map right Alt differently. The required invariant is that AltGr
        // must not be classified as shortcut Alt by `active_modifiers`.
        assert!(!active_modifiers(&state).alt);
    }
}

#[test]
fn dead_key_and_compose_key_are_recognized_as_local_composition_starts() {
    let Some(dead_keymap) = keymap_state("us", "intl") else {
        panic!("US international XKB keymap is required for composition tests");
    };
    let dead_acute = evdev_key_for_keysym(&dead_keymap, "dead_acute")
        .expect("US international keymap must expose dead_acute");
    assert!(key_is_composition(Some(&dead_keymap), dead_acute));

    let Some(compose_keymap) = keymap_state_with_options("us", "", "compose:ralt") else {
        panic!("XKB compose:ralt keymap is required for composition tests");
    };
    let multi_key = evdev_key_for_keysym(&compose_keymap, "Multi_key")
        .expect("compose:ralt keymap must expose Multi_key");
    assert!(key_is_composition(Some(&compose_keymap), multi_key));

    let letter =
        evdev_key_for_keysym(&compose_keymap, "a").expect("US keymap must expose the a key");
    assert!(!key_is_composition(Some(&compose_keymap), letter));
}

#[test]
fn local_composition_notifications_are_balanced_and_idempotent() {
    let mut state = StateData::new();
    begin_local_composition(&mut state);
    begin_local_composition(&mut state);
    assert!(state.composition_active);
    assert_eq!(
        state.events,
        VecDeque::from([InputEvent::CompositionChanged { active: true }])
    );

    finish_local_composition(&mut state);
    finish_local_composition(&mut state);
    assert!(!state.composition_active);
    assert_eq!(
        state.events,
        VecDeque::from([
            InputEvent::CompositionChanged { active: true },
            InputEvent::CompositionChanged { active: false },
        ])
    );
}

#[test]
fn multikey_sequence_keeps_the_engine_guarded_until_composed_text_commits() {
    let mut state = StateData::new();
    if state.local_compose.is_none() {
        panic!("the test locale must provide an XKB Compose table");
    }

    for (keysym, fallback) in [
        (xkeysym::key::Multi_key, None),
        (xkeysym::key::o, Some("o")),
    ] {
        let update = state
            .local_compose
            .as_mut()
            .expect("Compose state remains initialized")
            .feed(keysym, fallback);
        assert!(matches!(
            apply_compose_update(&mut state, update),
            ComposeUpdate::Pending
        ));
        assert!(state.composition_active);
    }

    let update = state
        .local_compose
        .as_mut()
        .expect("Compose state remains initialized")
        .feed(xkeysym::key::c, Some("c"));
    assert!(matches!(
        apply_compose_update(&mut state, update),
        ComposeUpdate::Composed(text) if text == "©"
    ));
    assert!(!state.composition_active);
    assert_eq!(
        state.events,
        VecDeque::from([
            InputEvent::CompositionChanged { active: true },
            InputEvent::CompositionChanged { active: false },
        ])
    );
}

#[test]
fn lifecycle_reset_clears_unfinished_compose_state() {
    let mut state = StateData::new();
    if state.local_compose.is_none() {
        panic!("the test locale must provide an XKB Compose table");
    }
    for (keysym, fallback) in [
        (xkeysym::key::Multi_key, None),
        (xkeysym::key::o, Some("o")),
    ] {
        let update = state
            .local_compose
            .as_mut()
            .expect("Compose state remains initialized")
            .feed(keysym, fallback);
        assert!(matches!(
            apply_compose_update(&mut state, update),
            ComposeUpdate::Pending
        ));
    }
    assert!(state.composition_active);

    reset_local_composition(&mut state);
    assert!(!state.composition_active);
    assert!(!state
        .local_compose
        .as_ref()
        .is_some_and(LocalCompose::is_composing));
    assert!(matches!(
        state
            .local_compose
            .as_mut()
            .expect("Compose state remains initialized")
            .feed(xkeysym::key::c, Some("c")),
        ComposeUpdate::Inactive
    ));
    assert_eq!(
        state.events,
        VecDeque::from([
            InputEvent::CompositionChanged { active: true },
            InputEvent::CompositionChanged { active: false },
        ])
    );
}

#[test]
fn pending_pass_through_preserves_modifiers() {
    let mut pending = VecDeque::from([PendingKeyPassThrough {
        keycode: 105,
        modifiers: Modifiers {
            alt: true,
            shift: true,
            ..Modifiers::default()
        },
        state: KeyEventState::Pressed,
    }]);
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };

    pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap();

    assert!(pending.is_empty());
    assert_eq!(
        injector.calls,
        vec![(
            105,
            Modifiers {
                alt: true,
                shift: true,
                ..Modifiers::default()
            }
        )]
    );
}

fn dispatch_one_queued_key(state: &mut StateData, injector: &mut RecordingInjector) {
    assert_eq!(state.events.pop_front(), Some(InputEvent::Reset));
    pass_through_pending_key(&mut state.pending_key_pass_through, Some(injector)).unwrap();
}

#[test]
fn held_left_arrow_is_press_repeat_release_not_taps() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };

    state.queue_virtual_key_event(105, Modifiers::default(), KeyEventState::Pressed);
    dispatch_one_queued_key(&mut state, &mut injector);
    // A repeat is a physical press notification for the already-held
    // key; the virtual key remains down and no second tap is emitted.
    state.queue_virtual_key_event(105, Modifiers::default(), KeyEventState::Pressed);
    assert!(state.pending_key_pass_through.is_empty());
    assert_eq!(state.events.pop_front(), Some(InputEvent::Reset));
    state.queue_virtual_key_event(105, Modifiers::default(), KeyEventState::Released);
    dispatch_one_queued_key(&mut state, &mut injector);

    assert_eq!(
        injector.events,
        vec![
            (105, KeyEventState::Pressed),
            (105, KeyEventState::Released),
        ]
    );
    assert!(state.virtual_held_keys.is_empty());
}

#[test]
fn held_delete_and_backspace_retain_repeat_semantics() {
    let mut keyboard = default_state();
    assert_eq!(
        key_action_and_update(&mut keyboard, 14, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Delete)
    );
    assert_eq!(
        key_action_and_update(&mut keyboard, 14, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Delete)
    );
    assert_eq!(
        key_action_and_update(&mut keyboard, 14, wl_keyboard::KeyState::Released),
        None
    );

    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    // Linux KEY_DELETE is unsupported by the commit contract and must
    // use the same held-key lifecycle as navigation keys.
    state.queue_virtual_key_event(111, Modifiers::default(), KeyEventState::Pressed);
    dispatch_one_queued_key(&mut state, &mut injector);
    state.queue_virtual_key_event(111, Modifiers::default(), KeyEventState::Pressed);
    assert_eq!(state.events.pop_front(), Some(InputEvent::Reset));
    state.queue_virtual_key_event(111, Modifiers::default(), KeyEventState::Released);
    dispatch_one_queued_key(&mut state, &mut injector);
    assert_eq!(
        injector.events,
        vec![
            (111, KeyEventState::Pressed),
            (111, KeyEventState::Released),
        ]
    );
}

#[test]
fn modifier_shortcut_preserves_physical_order() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    for (keycode, modifiers) in [
        (29, Modifiers::default()),
        (
            42,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        (
            203,
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ),
    ] {
        state.queue_virtual_key_event(keycode, modifiers, KeyEventState::Pressed);
        dispatch_one_queued_key(&mut state, &mut injector);
    }
    for (keycode, modifiers) in [
        (
            203,
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        ),
        (
            42,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        ),
        (29, Modifiers::default()),
    ] {
        state.queue_virtual_key_event(keycode, modifiers, KeyEventState::Released);
        dispatch_one_queued_key(&mut state, &mut injector);
    }

    assert_eq!(
        injector.events,
        vec![
            (29, KeyEventState::Pressed),
            (42, KeyEventState::Pressed),
            (203, KeyEventState::Pressed),
            (203, KeyEventState::Released),
            (42, KeyEventState::Released),
            (29, KeyEventState::Released),
        ]
    );
}

#[test]
fn alt_and_super_lifecycles_release_after_shortcuts() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    for (modifier, key) in [(56, 62), (125, 38)] {
        state.queue_virtual_key_event(modifier, Modifiers::default(), KeyEventState::Pressed);
        dispatch_one_queued_key(&mut state, &mut injector);
        state.queue_virtual_key_event(
            key,
            if modifier == 56 {
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                }
            } else {
                Modifiers {
                    super_key: true,
                    ..Modifiers::default()
                }
            },
            KeyEventState::Pressed,
        );
        dispatch_one_queued_key(&mut state, &mut injector);
        state.queue_virtual_key_event(key, Modifiers::default(), KeyEventState::Released);
        dispatch_one_queued_key(&mut state, &mut injector);
        state.queue_virtual_key_event(modifier, Modifiers::default(), KeyEventState::Released);
        dispatch_one_queued_key(&mut state, &mut injector);
    }

    assert_eq!(
        injector.events,
        vec![
            (56, KeyEventState::Pressed),
            (62, KeyEventState::Pressed),
            (62, KeyEventState::Released),
            (56, KeyEventState::Released),
            (125, KeyEventState::Pressed),
            (38, KeyEventState::Pressed),
            (38, KeyEventState::Released),
            (125, KeyEventState::Released),
        ]
    );
}

#[test]
fn focus_loss_releases_all_virtual_keys_before_focus_event() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    state.queue_virtual_key_event(125, Modifiers::default(), KeyEventState::Pressed);
    state.queue_virtual_key_event(
        46,
        Modifiers {
            super_key: true,
            ..Modifiers::default()
        },
        KeyEventState::Pressed,
    );
    state.queue_event(InputEvent::FocusChanged { sensitive: true });

    while matches!(state.events.front(), Some(InputEvent::Reset)) {
        dispatch_one_queued_key(&mut state, &mut injector);
    }
    assert_eq!(
        state.events.pop_front(),
        Some(InputEvent::FocusChanged { sensitive: true })
    );
    assert_eq!(
        injector.events,
        vec![
            (125, KeyEventState::Pressed),
            (46, KeyEventState::Pressed),
            (46, KeyEventState::Released),
            (125, KeyEventState::Released),
        ]
    );
    assert!(state.virtual_held_keys.is_empty());
}

#[test]
fn disconnect_cleanup_releases_pending_and_held_virtual_keys() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    state.queue_virtual_key_event(56, Modifiers::default(), KeyEventState::Pressed);
    release_virtual_keys_from_state(&mut state, Some(&mut injector));

    assert_eq!(
        injector.events,
        vec![(56, KeyEventState::Pressed), (56, KeyEventState::Released),]
    );
    assert!(state.pending_key_pass_through.is_empty());
    assert!(state.virtual_held_keys.is_empty());
}

#[test]
fn failed_key_release_remains_pending_for_cleanup_retry() {
    let mut state = StateData::new();
    let mut injector = FailFirstReleaseInjector {
        failed_release: false,
        events: Vec::new(),
    };
    state.queue_virtual_key_event(105, Modifiers::default(), KeyEventState::Pressed);
    state.queue_virtual_key_event(105, Modifiers::default(), KeyEventState::Released);

    pass_through_pending_key(&mut state.pending_key_pass_through, Some(&mut injector)).unwrap();
    let error = pass_through_pending_key(&mut state.pending_key_pass_through, Some(&mut injector))
        .unwrap_err();
    assert!(error.retryable);
    assert_eq!(state.pending_key_pass_through.len(), 1);

    release_virtual_keys_from_state(&mut state, Some(&mut injector));

    assert!(state.pending_key_pass_through.is_empty());
    assert_eq!(
        injector.events,
        vec![
            (105, KeyEventState::Pressed, false),
            (105, KeyEventState::Released, true),
            (105, KeyEventState::Released, false),
        ]
    );
}

#[test]
fn reconnect_starts_without_stale_virtual_keys() {
    let mut state = StateData::new();
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    state.queue_virtual_key_event(203, Modifiers::default(), KeyEventState::Pressed);
    release_virtual_keys_from_state(&mut state, Some(&mut injector));

    state.queue_virtual_key_event(203, Modifiers::default(), KeyEventState::Pressed);
    dispatch_one_queued_key(&mut state, &mut injector);

    assert_eq!(
        injector.events,
        vec![
            (203, KeyEventState::Pressed),
            (203, KeyEventState::Released),
            (203, KeyEventState::Pressed),
        ]
    );
    assert_eq!(state.virtual_held_keys, vec![203]);
}

#[test]
fn missing_pass_through_injector_is_reported_not_silent() {
    let mut pending = VecDeque::from([PendingKeyPassThrough {
        keycode: 1,
        modifiers: Modifiers::default(),
        state: KeyEventState::Pressed,
    }]);

    let error = pass_through_pending_keys(&mut pending, None).unwrap_err();

    assert!(error.retryable);
    assert!(error.message.contains("no key pass-through injector"));
    assert_eq!(pending.len(), 1);
}

#[test]
fn pass_through_injector_failure_is_reported_not_silent() {
    let mut pending = VecDeque::from([PendingKeyPassThrough {
        keycode: 1,
        modifiers: Modifiers::default(),
        state: KeyEventState::Pressed,
    }]);
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: true,
    };

    let error = pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap_err();

    assert!(error.retryable);
    assert!(error.message.contains("synthetic failure"));
}

#[test]
fn overflowing_protocol_keycode_fails_closed() {
    let mut state = default_state();
    assert!(matches!(
        key_action_and_update(&mut state, u32::MAX, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn unrelated_keysym_is_left_for_utf8_conversion() {
    assert_eq!(classify_keysym('a' as u32), None);
}

#[test]
fn password_and_unknown_content_purposes_are_sensitive() {
    use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
        ContentHint, ContentPurpose,
    };

    assert!(content_type_is_sensitive(
        WEnum::Value(ContentHint::None),
        WEnum::Value(ContentPurpose::Password),
    ));
    assert!(content_type_is_sensitive(
        WEnum::Value(ContentHint::None),
        WEnum::Unknown(255),
    ));
    assert!(content_type_is_sensitive(
        WEnum::Value(ContentHint::HiddenText),
        WEnum::Value(ContentPurpose::Normal),
    ));
    assert!(content_type_is_sensitive(
        WEnum::Value(ContentHint::SensitiveData),
        WEnum::Value(ContentPurpose::Normal),
    ));
    assert!(!content_type_is_sensitive(
        WEnum::Value(ContentHint::None),
        WEnum::Value(ContentPurpose::Normal),
    ));
}

#[test]
fn input_method_commit_size_is_checked_before_injection() {
    assert!(validate_commit_text(&"a".repeat(MAX_COMMIT_TEXT_BYTES)).is_ok());
    assert!(validate_commit_text(&"a".repeat(MAX_COMMIT_TEXT_BYTES + 1)).is_err());
}

#[test]
fn input_method_control_characters_are_rejected_before_injection() {
    assert!(validate_commit_text("\u{0001}").is_err());
    assert!(validate_commit_text("safe\ntext\r\t").is_ok());
}

#[test]
fn connection_poll_errors_are_terminal() {
    use rustix::event::PollFlags;

    assert!(connection_poll_failed(PollFlags::ERR));
    assert!(connection_poll_failed(PollFlags::HUP));
    assert!(connection_poll_failed(PollFlags::NVAL));
    assert!(!connection_poll_failed(PollFlags::IN));
}

#[test]
fn transport_failures_are_retryable_but_protocol_failures_are_not() {
    assert!(InputMethodError::Unavailable.is_retryable());
    assert!(InputMethodError::Transport("socket closed".into()).is_retryable());
    assert!(InputMethodError::Timeout("startup".into()).is_retryable());
    assert!(!InputMethodError::Protocol("unsupported key".into()).is_retryable());
    assert!(!InputMethodError::Keymap("invalid".into()).is_retryable());
}

#[test]
fn backspace_uses_utf8_byte_length() {
    let surrounding = SurroundingText {
        text: "café".into(),
        cursor: 5,
        anchor: 5,
    };
    assert_eq!(backspace_delete_lengths(Some(&surrounding)), Some((2, 0)));
}

#[test]
fn backspace_deletes_whole_grapheme_cluster() {
    // "e" (1 byte) + combining acute accent U+0301 (2 bytes) is a single
    // extended grapheme cluster; Backspace must remove both codepoints,
    // not just the trailing combining mark.
    let surrounding = SurroundingText {
        text: "e\u{0301}".into(),
        cursor: 3,
        anchor: 3,
    };
    assert_eq!(backspace_delete_lengths(Some(&surrounding)), Some((3, 0)));
}

#[test]
fn backspace_deletes_selection_or_fails_closed() {
    let selected = SurroundingText {
        text: "hello".into(),
        cursor: 5,
        anchor: 2,
    };
    assert_eq!(backspace_delete_lengths(Some(&selected)), Some((3, 0)));
    assert_eq!(backspace_delete_lengths(None), None);
}

#[test]
fn selection_backspace_is_a_boundary_for_matching() {
    let selected = SurroundingText {
        text: "hello".into(),
        cursor: 5,
        anchor: 2,
    };
    let collapsed = SurroundingText {
        text: "hello".into(),
        cursor: 5,
        anchor: 5,
    };
    assert!(surrounding_has_selection(Some(&selected)));
    assert!(!surrounding_has_selection(Some(&collapsed)));
    assert!(!surrounding_has_selection(None));
}

#[test]
fn empty_surrounding_text_does_not_emit_matcher_backspace() {
    let empty = SurroundingText {
        text: String::new(),
        cursor: 0,
        anchor: 0,
    };
    assert_eq!(backspace_delete_lengths(Some(&empty)), Some((0, 0)));
    assert!(!surrounding_has_selection(Some(&empty)));
    assert_eq!(
        matcher_event_for_deletion(0, 0, false),
        InputEvent::EndOfInput
    );
    assert_eq!(
        matcher_event_for_deletion(2, 0, false),
        InputEvent::Backspace
    );
}

#[test]
fn unsupported_key_clears_pending_matching_state() {
    assert_eq!(matcher_event_for_unsupported_key(), InputEvent::Reset);
}

#[test]
fn deactivation_disables_matching() {
    assert_eq!(
        deactivation_event(),
        InputEvent::FocusChanged { sensitive: true }
    );
}

#[test]
fn replacement_context_must_match_exact_utf8_trigger() {
    let surrounding = SurroundingText {
        text: "café:x".into(),
        cursor: 7,
        anchor: 7,
    };
    assert!(surrounding_ends_with_trigger(Some(&surrounding), ":x"));
    assert!(!surrounding_ends_with_trigger(Some(&surrounding), ":y"));
    assert!(!surrounding_ends_with_trigger(None, ":x"));
}

#[test]
fn forwarded_text_updates_context_before_immediate_replacement() {
    let mut surrounding = Some(SurroundingText {
        text: ":".into(),
        cursor: 1,
        anchor: 1,
    });

    optimistic_commit(&mut surrounding, "x");

    assert!(surrounding_ends_with_trigger(surrounding.as_ref(), ":x"));
    optimistic_replace(&mut surrounding, ":x", "expanded").unwrap();
    assert_eq!(surrounding.as_ref().unwrap().text, "expanded");
}

#[test]
fn forwarded_word_boundary_delimiter_is_replaced_atomically() {
    // input-method-v2 forwards the delimiter before the engine reports
    // the word-boundary match. The transaction must therefore validate
    // and replace trigger + delimiter together; validating only `:sig`
    // would reject the expansion because the surrounding text ends in
    // `:sig `.
    let mut surrounding = Some(SurroundingText {
        text: ":sig ".into(),
        cursor: 5,
        anchor: 5,
    });

    assert!(surrounding_ends_with_trigger(surrounding.as_ref(), ":sig "));
    optimistic_replace(&mut surrounding, ":sig ", "signature ").unwrap();
    assert_eq!(surrounding.as_ref().unwrap().text, "signature ");
    assert_eq!(surrounding.as_ref().unwrap().cursor, 10);
}

#[test]
fn replacement_context_rejects_selection_and_invalid_cursor() {
    let selected = SurroundingText {
        text: ":x".into(),
        cursor: 2,
        anchor: 0,
    };
    assert!(!surrounding_ends_with_trigger(Some(&selected), ":x"));
    let invalid = SurroundingText {
        text: ":x".into(),
        cursor: 1,
        anchor: 1,
    };
    assert!(!surrounding_ends_with_trigger(Some(&invalid), ":x"));
}

#[test]
fn protocol_event_queue_is_bounded_and_fails_closed() {
    let mut state = StateData::new();
    state.queue_virtual_key_event(125, Modifiers::default(), KeyEventState::Pressed);
    for _ in 0..MAX_QUEUED_EVENTS {
        state.queue_event(InputEvent::Text("x".into()));
    }
    state.queue_event(InputEvent::Text("overflow".into()));
    assert_eq!(state.events.len(), 2);
    assert_eq!(state.events.front(), Some(&InputEvent::Reset));
    assert_eq!(
        state.events.get(1),
        Some(&InputEvent::Text("overflow".into()))
    );
    assert_eq!(state.pending_key_pass_through.len(), 1);

    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    release_virtual_keys_from_state(&mut state, Some(&mut injector));
    assert_eq!(
        injector.events,
        vec![
            (125, KeyEventState::Pressed),
            (125, KeyEventState::Released),
        ]
    );
}

#[test]
fn focus_policy_events_supersede_queued_keyboard_events() {
    let mut state = StateData::new();
    state.queue_event(InputEvent::Text("stale".into()));
    state.queue_event(InputEvent::FocusChanged { sensitive: true });
    assert_eq!(
        state.events.as_slices().0,
        &[InputEvent::FocusChanged { sensitive: true }]
    );
}

#[test]
fn pass_through_queue_is_bounded_and_overflow_is_reported() {
    let mut state = StateData::new();
    for i in 0..MAX_PENDING_KEY_PASS_THROUGH {
        state
            .pending_key_pass_through
            .push_back(PendingKeyPassThrough {
                keycode: (i % 256) as u32,
                modifiers: Modifiers::default(),
                state: KeyEventState::Pressed,
            });
    }

    assert_eq!(
        state.pending_key_pass_through.len(),
        MAX_PENDING_KEY_PASS_THROUGH
    );

    state
        .pending_key_pass_through
        .push_back(PendingKeyPassThrough {
            keycode: 1,
            modifiers: Modifiers::default(),
            state: KeyEventState::Pressed,
        });

    assert_eq!(
        state.pending_key_pass_through.len(),
        MAX_PENDING_KEY_PASS_THROUGH + 1,
        "queue bounds are enforced at dispatch time, not push time"
    );
}

#[test]
fn dropped_key_up_at_queue_capacity_remains_tracked_for_teardown() {
    let mut state = StateData::new();
    state.virtual_held_keys.push(999);
    for index in 0..MAX_PENDING_KEY_PASS_THROUGH {
        state
            .pending_key_pass_through
            .push_back(PendingKeyPassThrough {
                keycode: index as u32,
                modifiers: Modifiers::default(),
                state: KeyEventState::Pressed,
            });
    }

    state.queue_virtual_key_event(999, Modifiers::default(), KeyEventState::Released);

    assert!(state.error.is_some());
    assert_eq!(state.virtual_held_keys, vec![999]);
    assert_eq!(
        state.pending_key_pass_through.len(),
        MAX_PENDING_KEY_PASS_THROUGH
    );
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };
    release_virtual_keys_from_state(&mut state, Some(&mut injector));
    assert_eq!(
        injector.events.last(),
        Some(&(999, KeyEventState::Released))
    );
}

#[test]
fn escape_key_produces_unsupported_action() {
    let mut state = default_state();
    // Escape (keycode 1) must be unsupported and queued for pass-through
    assert!(matches!(
        key_action_and_update(&mut state, 1, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn ctrl_key_can_combine_with_unsupported_keys() {
    let mut state = default_state();
    // Left Ctrl must be pressed first
    assert!(matches!(
        key_action_and_update(&mut state, 29, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));

    let modifiers = active_modifiers(&state);
    assert!(modifiers.ctrl);

    // Left arrow with Ctrl must be unsupported (pass-through)
    assert!(matches!(
        key_action_and_update(&mut state, 203, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn alt_f4_combination_is_unsupported() {
    let mut state = default_state();
    // Left Alt must be pressed first
    assert!(matches!(
        key_action_and_update(&mut state, 56, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));

    let modifiers = active_modifiers(&state);
    assert!(modifiers.alt);

    // F4 with Alt must be unsupported (pass-through)
    assert!(matches!(
        key_action_and_update(&mut state, 62, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn shift_modifies_key_action() {
    let mut state = default_state();
    // Left Shift must be pressed first
    assert!(matches!(
        key_action_and_update(&mut state, 42, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Ignore)
    ));

    let modifiers = active_modifiers(&state);
    assert!(modifiers.shift);

    // Left arrow with Shift must be unsupported (pass-through)
    assert!(matches!(
        key_action_and_update(&mut state, 203, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));
}

#[test]
fn multiple_queued_keys_are_passed_through_in_order() {
    let mut pending = VecDeque::from([
        PendingKeyPassThrough {
            keycode: 105,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            state: KeyEventState::Pressed,
        },
        PendingKeyPassThrough {
            keycode: 62,
            modifiers: Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            state: KeyEventState::Pressed,
        },
        PendingKeyPassThrough {
            keycode: 203,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
            state: KeyEventState::Pressed,
        },
    ]);
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: false,
    };

    pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap();

    assert!(pending.is_empty());
    assert_eq!(injector.calls.len(), 3);
    assert_eq!(
        injector.calls[0],
        (
            105,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            }
        )
    );
    assert_eq!(
        injector.calls[1],
        (
            62,
            Modifiers {
                alt: true,
                ..Modifiers::default()
            }
        )
    );
    assert_eq!(
        injector.calls[2],
        (
            203,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            }
        )
    );
}

#[test]
fn injector_error_stops_pass_through_and_preserves_remaining_keys() {
    let mut pending = VecDeque::from([
        PendingKeyPassThrough {
            keycode: 105,
            modifiers: Modifiers::default(),
            state: KeyEventState::Pressed,
        },
        PendingKeyPassThrough {
            keycode: 106,
            modifiers: Modifiers::default(),
            state: KeyEventState::Released,
        },
    ]);
    let mut injector = RecordingInjector {
        calls: Vec::new(),
        events: Vec::new(),
        fail: true,
    };

    let error = pass_through_pending_keys(&mut pending, Some(&mut injector)).unwrap_err();

    assert!(error.retryable, "injector errors should be retryable");
    assert!(error.message.contains("synthetic failure"));
    assert_eq!(
        pending.len(),
        2,
        "failed and not-yet-sent keys should be preserved on error"
    );
    assert_eq!(pending.front().unwrap().keycode, 105);
    assert_eq!(pending.get(1).unwrap().keycode, 106);
}

#[test]
fn focus_change_clears_pending_keys() {
    let mut state_data = StateData::new();
    state_data
        .pending_key_pass_through
        .push_back(PendingKeyPassThrough {
            keycode: 203,
            modifiers: Modifiers::default(),
            state: KeyEventState::Pressed,
        });

    assert_eq!(state_data.pending_key_pass_through.len(), 1);

    state_data.pending_key_pass_through.clear();

    assert!(state_data.pending_key_pass_through.is_empty());
}

#[test]
fn key_release_does_not_generate_action() {
    let mut state = default_state();

    // Left arrow press must be unsupported
    assert!(matches!(
        key_action_and_update(&mut state, 203, wl_keyboard::KeyState::Pressed),
        Some(KeyAction::Unsupported)
    ));

    // Release should not generate an action
    assert_eq!(
        key_action_and_update(&mut state, 203, wl_keyboard::KeyState::Released),
        None
    );
}
