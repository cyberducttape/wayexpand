# Troubleshooting

Start with a non-invasive snapshot:

```sh
wayexpand --version
wayexpand doctor
wayexpand status --json
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 80 --no-pager
```

## `configuration invalid`

Run `wayexpand doctor`. Common causes are malformed TOML, duplicate triggers,
a file writable by group or other users, an untrusted parent directory, or a
non-regular path. Fix the reported mode/owner and validate again. Never bypass
the check with `chmod 777`.

## Service is active but snippets do not expand

Check `wayexpand status`, `wayexpand backend`, and `echo "$WAYLAND_DISPLAY"`.
The service may be using the stdin harness, may be reconnecting, or may be
running on a compositor without input-method-v2 support. Confirm that the
input-method unit—not the harness unit—is enabled and that `state=connected`.

## `control socket unavailable`

Ensure `XDG_RUNTIME_DIR` is set and points to a user-owned runtime directory.
If using `WAYEXPAND_SOCKET`, its parent must exist, be a directory, and not be
group/world-writable. Remove only a stale socket owned by the current user; the
daemon deliberately refuses to remove arbitrary files.

## Reload was rejected

The daemon keeps the previous working configuration by design. Fix the file and
request another reload:

```sh
wayexpand validate
wayexpand reload
wayexpand status
```

If the editor is saving repeatedly, wait for its atomic save to finish and
retry. An unstable or half-written file is not activated.

## GUI will not start

Use the terminal UI to separate configuration problems from graphics/session
problems:

```sh
wayexpand-ui
wayexpand doctor
```

Confirm `WAYLAND_DISPLAY` is present and try launching from the same graphical
session. On a headless machine, use the CLI or terminal UI.

## Output backend reconnects

Transport failures are retried with bounded backoff. The failed replacement is
not replayed because the compositor may have accepted part of it. Once
`state=connected` returns, type a fresh trigger. Permanent protocol or
validation errors require operator action and are not retried indefinitely.

## KDE Plasma (KWin) - app_filter / window tracking issues

### Window tracking unavailable or unreliable

App-scoped expansions (`app_filter`) rely on KWin's scripting interface to
track the focused window. This is a KDE privacy choice: no Wayland protocol
exposes window identity, so we use KWin's `org.kde.kwin.Scripting` D-Bus API.

**Symptoms:**
- `wayexpand doctor` reports "window tracker not reachable"
- App filters are silently ignored (fail-closed behavior)

**Causes:**
- KWin < 6.0 or KWin built without scripting support
- D-Bus session bus connectivity issues
- KWin scripting engine crashed or reloaded

**Solution:**
1. Verify KWin version: `kwin_wayland --version` should be 6.0+
2. Check D-Bus: `dbus-send --session --print-reply --dest=org.kde.KWin /Scripting org.freedesktop.DBus.Introspectable.Introspect`
3. Restart KWin if the above fails: Log out and back in, or restart the session

### KWin script registration delay

When enabling app filters, you may see a 1-2 second delay before the GUI
responds to "Use current app". This is a known race condition:

**Root cause:**
- `org.kde.kwin.Scripting.loadScript()` returns before the `/Scripting/ScriptN`
  D-Bus object is fully registered on the session bus
- WayExpand retries the `run()` call up to 15 times with 150ms delays (total
  ~2.25 seconds) to work around this

**Solution:**
- This is expected. The delay only happens on first enable; subsequent calls
  are instant.
- To adjust retry behavior (advanced users), rebuild with different constants
  in `crates/backend-kwin-window/src/lib.rs`:
  ```rust
  const LOAD_RETRY_ATTEMPTS: u32 = 15;      // Number of attempts
  const LOAD_RETRY_DELAY: Duration = Duration::from_millis(150);  // Delay between attempts
  ```

### Stale KWin scripts after daemon crash

If the WayExpand daemon is killed ungracefully (`kill -9`), leftover KWin
scripts may remain:
- D-Bus service: `org.wayexpand.WindowTracker.pid<PID>` (unregistered but gone)
- Script file: `/tmp/wayexpand-window-tracker-<PID>.js` (orphaned)

**Impact:** Low — subsequent daemon restarts use a different PID and register a
new service/script with no collision.

**Cleanup (if concerned):**
```sh
rm /tmp/wayexpand-window-tracker-*.js
```

Normal daemon shutdown cleans these up automatically.

## Wlroots Compositors (Sway, Hyprland) - Coming in 1.1

Window tracking (`app_filter`) is not yet implemented for wlroots compositors.
The feature is planned for 1.1 using the standard `wlr-foreign-toplevel-management`
protocol.

**Workaround:** Disable app filters until 1.1, or use global expansions only.

**Tracking:** See the [1.0 checklist](../RELEASE_1.0_CHECKLIST.md#5-this-sessions-own-follow-through-items).
