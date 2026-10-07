use super::*;
use wayland_client::{Dispatch, QueueHandle};

impl Dispatch<wl_registry::WlRegistry, ()> for StateData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == "zwp_input_method_manager_v2" && state.manager.is_none() {
            state.manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == "wl_seat" && state.seat.is_none() {
            state.seat = Some(registry.bind(name, version.min(7), qh, ()));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for StateData {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.initial_roundtrip_done = true;
        }
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for StateData {
    fn event(
        _: &mut Self,
        _: &ZwpInputMethodManagerV2,
        _: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for StateData {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wayland_client::protocol::wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for StateData {
    fn event(
        state: &mut Self,
        proxy: &ZwpInputMethodV2,
        event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event,
        _: &(),
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Activate => {
                // The compositor may activate without an intervening
                // Deactivate; release the outgoing grab object so it does not
                // leak on the compositor side.
                if let Some(previous) = state.keyboard.take() {
                    previous.release();
                    let _ = connection.flush();
                }
                state.keyboard = Some(proxy.grab_keyboard(qh, ()));
                state.activate();
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Deactivate => {
                if let Some(previous) = state.keyboard.take() {
                    previous.release();
                    let _ = connection.flush();
                }
                state.deactivate();
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::ContentType { hint, purpose } => {
                state.set_content_type(hint, purpose);
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                state.surrounding_text = Some(SurroundingText {
                    text,
                    cursor,
                    anchor,
                });
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Done => {
                state.finish_protocol_batch();
            }
            wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event::Unavailable => {
                state.error = Some(InputMethodError::Unavailable);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for StateData {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Deactivation can race with events already queued by the compositor.
        // Do not commit or delete anything from a stale exclusive grab.
        if state.keyboard.is_none() {
            return;
        }
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Keymap { format, fd, size } => {
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    state.error = Some(InputMethodError::Keymap(format!(
                        "unsupported keymap format {format:?}"
                    )));
                    return;
                }
                match decode_keymap(fd, size) {
                    Ok(keymap) => {
                        reset_local_composition(state);
                        state.keyboard_state = Some(State::new(keymap));
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keyboard_state) = state.keyboard_state.as_mut() {
                    keyboard_state.update_mask(
                        mods_depressed,
                        mods_latched,
                        mods_locked,
                        0,
                        0,
                        group as usize,
                    );
                }
            }
            Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                let composition_key = key_is_composition(state.keyboard_state.as_ref(), key);
                let raw_keysym = state
                    .keyboard_state
                    .as_ref()
                    .and_then(|keyboard_state| {
                        key.checked_add(8)
                            .and_then(|code| keyboard_state.key_get_one_sym(code))
                    })
                    .map(|keysym| keysym.raw());
                let is_modifier = state
                    .keyboard_state
                    .as_ref()
                    .is_some_and(|keyboard_state| key_is_modifier(keyboard_state, key));
                let modifiers = state
                    .keyboard_state
                    .as_ref()
                    .map(active_modifiers)
                    .unwrap_or_default();
                if key_state == wl_keyboard::KeyState::Pressed {
                    if let Some(keyboard_state) = state.keyboard_state.as_ref() {
                        if let Some(chord) = key_chord(keyboard_state, key) {
                            state.queue_event(InputEvent::Key(chord));
                        }
                    }
                }
                let was_held = state.virtual_held_keys.contains(&key);
                let action = state
                    .keyboard_state
                    .as_mut()
                    .map_or(Some(KeyAction::Unsupported), |keyboard_state| {
                        key_action_and_update(keyboard_state, key, key_state)
                    });
                let fallback_text = action.as_ref().and_then(|action| match action {
                    KeyAction::Text(text) => Some(text.as_str()),
                    KeyAction::Commit(text) => Some(*text),
                    _ => None,
                });
                let safe_compose_modifier = raw_keysym.is_some_and(|keysym| {
                    matches!(keysym, xkeysym::key::Shift_L | xkeysym::key::Shift_R)
                });
                let compose_active = state
                    .local_compose
                    .as_ref()
                    .is_some_and(LocalCompose::is_composing);
                let compose_update = if key_state == wl_keyboard::KeyState::Pressed && !is_modifier
                {
                    raw_keysym.and_then(|keysym| {
                        state
                            .local_compose
                            .as_mut()
                            .map(|compose| compose.feed(keysym, fallback_text))
                    })
                } else {
                    None
                };
                let compose_update = if is_modifier && !safe_compose_modifier && compose_active {
                    state
                        .local_compose
                        .as_mut()
                        .map(|compose| ComposeUpdate::Cancelled(compose.cancel()))
                } else {
                    compose_update
                };
                match compose_update.map(|update| apply_compose_update(state, update)) {
                    Some(ComposeUpdate::Pending) => {
                        return;
                    }
                    Some(ComposeUpdate::Composed(text)) => {
                        if !text.is_empty() {
                            forward_commit(state, connection, &text);
                            state.queue_event(InputEvent::Text(text));
                        }
                        return;
                    }
                    Some(ComposeUpdate::Cancelled(fallback)) => {
                        if !fallback.is_empty() {
                            forward_commit(state, connection, &fallback);
                            state.queue_event(InputEvent::Text(fallback));
                        }
                        if matches!(action, Some(KeyAction::Text(_) | KeyAction::Commit(_))) {
                            return;
                        }
                    }
                    Some(ComposeUpdate::Inactive) | None => {
                        if key_state == wl_keyboard::KeyState::Pressed && composition_key {
                            begin_local_composition(state);
                        }
                    }
                }
                if is_modifier && compose_active && safe_compose_modifier {
                    return;
                }
                if is_modifier
                    || matches!(action, Some(KeyAction::Unsupported))
                    || (key_state == wl_keyboard::KeyState::Released && was_held)
                {
                    state.queue_virtual_key_event(
                        key,
                        modifiers,
                        match key_state {
                            wl_keyboard::KeyState::Pressed => KeyEventState::Pressed,
                            wl_keyboard::KeyState::Released => KeyEventState::Released,
                            _ => unreachable!("unknown key state is handled by a separate arm"),
                        },
                    );
                    return;
                }
                match action {
                    Some(KeyAction::Delete) => {
                        let selected = surrounding_has_selection(state.surrounding_text.as_ref());
                        if let Some((before, after)) =
                            backspace_delete_lengths(state.surrounding_text.as_ref())
                        {
                            forward_delete(state, connection, before, after);
                            state
                                .events
                                .push_back(matcher_event_for_deletion(before, after, selected));
                        } else {
                            state.error = Some(InputMethodError::Protocol(
                                "cannot safely forward Backspace without valid surrounding text"
                                    .into(),
                            ));
                        }
                    }
                    Some(KeyAction::Commit(text)) => {
                        finish_local_composition(state);
                        forward_commit(state, connection, text);
                        state.queue_event(InputEvent::Delimiter(
                            text.chars().next().unwrap_or('\n'),
                        ));
                    }
                    Some(KeyAction::Text(text)) => {
                        finish_local_composition(state);
                        forward_commit(state, connection, &text);
                        state.queue_event(InputEvent::Text(text));
                    }
                    Some(KeyAction::Ignore) => {}
                    Some(KeyAction::Unsupported) => {
                        unreachable!("unsupported keys are handled above")
                    }
                    None => {}
                }
            }
            Event::Key {
                state: WEnum::Unknown(_),
                ..
            } => {
                // An unrecognized key-state enum is a single malformed event,
                // not a reason to tear down the whole source: discard it the
                // same way an unsupported key is discarded.
                state.queue_event(matcher_event_for_unsupported_key());
            }
            _ => {}
        }
    }
}
