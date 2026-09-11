# Wayland integration testing

The repository CI validates the platform-independent engine, protocol-safe
backend logic, systemd units, and daemon lifecycle. A real compositor is still
required for end-to-end keyboard behavior.

## Common preparation

Build the workspace and run the compositor-independent checks:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
bash scripts/smoke-daemon.sh
wayexpand doctor
```

Record the compositor, desktop session, keyboard layout, and output of
`wayexpand doctor` for every integration run.

## Input-method source

In a session that advertises `zwp_input_method_manager_v2`:

```sh
wayexpand-daemon --source=input-method /path/to/expansions.toml
```

Verify each item in a normal text editor, terminal, browser, and a native
toolkit application where available:

1. ASCII expansion replaces the complete trigger.
2. UTF-8 replacement works for accented text, CJK, emoji, and punctuation.
3. Backspace removes one Unicode scalar and selections are handled correctly.
4. Return and Tab clear the pending trigger buffer.
5. Password and hidden-text fields do not capture or expand.
6. Unsupported non-text keys are not interpreted as text, clear the pending
   trigger, and do not restart the daemon. Because input-method-v2 gives the
   backend an exclusive grab, the individual unsupported event may be lost;
   this is a known limitation until compositor-specific pass-through is
   implemented.
7. Compositor restart causes input-method recovery with bounded backoff; a
   non-retryable protocol failure still stops visibly and systemd applies its
   bounded restart policy.

Capture `systemctl --user status wayexpand-input-method.service` and relevant
user-journal output after each failure. Do not include typed secrets or
replacement contents in reports.

Do not substitute a GNOME or KDE session for this prerequisite. The protocol
is experimental and current compositor support is not uniform; a failed probe
is a capability result, not a configuration error. For desktops that do not
expose this protocol, test the explicit libei/RemoteDesktop backend instead.

## wlroots output

In a compositor exposing `zwp_virtual_keyboard_v1`, first test the isolated
output path:

```sh
cargo run -p wayexpand-backend-wlroots --bin wlroots-type -- 'Hello 🙂'
```

Then exercise the stdin harness with `--backend=wlroots`. This validates output
only; it is not evidence of global input capture.

## libei output

For a direct EIS endpoint:

```sh
LIBEI_SOCKET=wayland-0-eis \
  wayexpand-daemon --backend=libei /path/to/expansions.toml
```

For the portal path, omit `LIBEI_SOCKET` and explicitly select `--backend=libei`.
Record consent, device capability negotiation, UTF-8 insertion, Backspace, and
behavior after portal revocation or compositor restart.

## Evidence boundary

Passing the repository smoke test proves daemon lifecycle behavior only. It does
not prove compositor protocol availability, input-method activation, keyboard
pass-through, preedit support, or desktop-wide compatibility.
