# Troubleshooting WayExpand

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Troubleshooting**

---

Start with a non-invasive diagnostic snapshot:

```sh
wayexpand --version
wayexpand doctor
wayexpand status --json
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 80 --no-pager
```

## Common Issues

### Configuration invalid

**Error:** `wayexpand: configuration invalid`

**Solutions:**
1. Run `wayexpand doctor` to see the specific error
2. Validate TOML syntax: `wayexpand validate ~/.config/wayexpand/expansions.toml`
3. Check file permissions: `ls -la ~/.config/wayexpand/expansions.toml` (should be 0600)

**Common causes:**
- Malformed TOML (missing quotes, commas, brackets)
- Duplicate triggers in the same config file
- File writable by group or other users (use `chmod 600`)
- Untrusted parent directory or symlink
- Non-regular file (directory instead of file)

**Never bypass security checks with `chmod 777`** — fix the actual issue.

### Service is active but snippets do not expand

**Symptoms:** Daemon is running, but triggers don't work

**Diagnosis:**
```sh
wayexpand status           # Check connection state
wayexpand doctor           # Verify backend selection
echo "$WAYLAND_DISPLAY"    # Confirm Wayland session
```

**Common causes:**
- Using stdin harness instead of input-method-v2
- Daemon is reconnecting to compositor
- Compositor doesn't support input-method-v2 (try `wayexpand doctor` to see what's available)
- Wrong backend selected (check `wayexpand status`)

**Solution:** Verify `state=connected` and that the input-method unit (not the harness) is enabled:
```sh
systemctl --user status wayexpand-input-method.service
systemctl --user enable wayexpand-input-method.service  # if needed
systemctl --user restart wayexpand-input-method.service
```

### Control socket unavailable

**Error:** `control socket unavailable`

**Solutions:**
1. Ensure `XDG_RUNTIME_DIR` is set: `echo $XDG_RUNTIME_DIR`
2. Verify it's user-owned: `ls -la $XDG_RUNTIME_DIR`
3. Check permissions (should not be group/world-writable): `stat $XDG_RUNTIME_DIR`

**If using custom socket path with `WAYEXPAND_SOCKET`:**
- Parent directory must exist and be a directory
- Parent must not be group or world-writable
- Current user must own the directory

**Cleanup stale socket (only if owned by you):**
```sh
rm $XDG_RUNTIME_DIR/wayexpand.sock  # or $WAYEXPAND_SOCKET
systemctl --user restart wayexpand-input-method.service
```

### Reload was rejected

**Situation:** Changed config file, but reload failed

**Why this is safe:** The daemon keeps the previous working configuration by design. Bad configs never become active.

**Fix it:**
```sh
wayexpand validate                              # Find the error
# Edit the file to fix the problem
wayexpand reload                                # Try again
wayexpand status                                # Verify it worked
```

**Tip:** If your editor saves repeatedly, wait for atomic writes to finish before retrying.

### GUI will not start

**Error:** `wayexpand-gui` crashes or doesn't appear

**Diagnosis:**
```sh
wayexpand-ui              # Try the terminal UI instead
wayexpand doctor          # Check if graphics are the issue
echo $WAYLAND_DISPLAY     # Confirm Wayland session
```

**Common causes:**
- Graphics/rendering issues (try `wayexpand-ui` as fallback)
- Wrong session environment
- Headless machine (no display)
- Missing font or theme data

**Solutions:**
- Use the terminal UI (`wayexpand-ui`) as a workaround
- Ensure you're running in the graphical session: `echo $WAYLAND_DISPLAY` should be set
- Check journal for graphics errors: `journalctl --user -n 50`

### Output backend reconnects

**Situation:** Replacements are being re-injected or connection keeps breaking

**Why this happens:** Transport failures are retried with bounded backoff. Failed replacements are NOT replayed because the compositor may have partially accepted them.

**What to do:**
1. Wait for `state=connected` to return:
```sh
wayexpand doctor  # Checks connection status
```
2. Type a fresh trigger (don't retry the same one)
3. If it keeps reconnecting, check the logs:
```sh
journalctl --user -u wayexpand-input-method.service -f
```

**Permanent errors** (validation or protocol problems) require operator action and are not retried indefinitely. Check `wayexpand doctor` output for what's wrong.

## Compositor-Specific Issues

### KDE Plasma (KWin) — app_filter / window tracking issues

#### Window tracking unavailable

**Symptoms:**
- `wayexpand doctor` reports "window tracker not reachable"
- App filters are silently ignored (fail-closed)

**Causes:**
- KWin < 6.0 or built without scripting support
- D-Bus session bus connectivity issues
- KWin scripting engine crashed or was reloaded

**Fix:**
1. Check KWin version: `kwin_wayland --version` (should be 6.0+)
2. Test D-Bus connectivity:
```sh
dbus-send --session --print-reply --dest=org.kde.KWin /Scripting \
  org.freedesktop.DBus.Introspectable.Introspect
```
3. If that fails, restart KWin: log out and back in

#### KWin script registration delay

**Symptom:** When enabling app filters, 1-2 second delay before GUI responds to "Use current app"

**Why it happens:** The `loadScript()` call returns before the D-Bus object is fully registered

**This is expected.** The delay only happens on first enable; subsequent calls are instant.

**Advanced:** To adjust retry behavior, rebuild with different constants in `crates/backend-kwin-window/src/lib.rs`:
```rust
const LOAD_RETRY_ATTEMPTS: u32 = 15;                          // Number of attempts
const LOAD_RETRY_DELAY: Duration = Duration::from_millis(150); // Delay between attempts
```

#### Stale KWin scripts after daemon crash

**What can happen:** If the daemon is killed with `kill -9`, KWin scripts may remain

**Impact:** Low — script filenames use random nonces to prevent collisions

**Automatic cleanup:** Normal daemon shutdown cleans these up automatically

**Manual cleanup (if concerned):**
```sh
ls /tmp/wayexpand-window-tracker-*.js 2>/dev/null || echo "None found"
# Safe to delete these manually if they exist
```

### Wlroots Compositors (Sway, Hyprland, river)

#### App_filter not supported yet

**Status:** Window tracking is not yet implemented for wlroots compositors

**Planned:** Standard `wlr-foreign-toplevel-management` protocol will enable app_filter on Sway, Hyprland, and other wlroots-based compositors

**Workaround:** Use global expansions instead of app filters. App filters fail closed (never match) if window tracking is unavailable, so they're safe to enable — they just won't provide filtering.

### GNOME

#### App_filter not supported

**Why:** GNOME/Mutter has no window-tracking protocol available

**Workaround:** Use global hotkeys or trigger-based expansions instead of app-filtered triggers

**Expected:** Future versions may support alternative window tracking mechanisms

## Network and Permissions

### Evdev permissions issues

**Symptom:** Snippets don't expand when using `--source=evdev`

Evdev is a best-effort compatibility backend. Because it observes keyboard
events non-exclusively, very fast typing can race with trigger replacement and
produce an unexpanded trigger or overlapping text. The quiet period reduces
this risk but cannot make replacement atomic; use an interception-capable
protocol when that guarantee is required.

**Check the active-seat ACL (default):**
```sh
getfacl /dev/input/eventN  # Look for an ACL entry for the active user
wayexpand doctor
```

**Fix (default active-seat mode):**
```sh
sudo ./scripts/install-evdev-permissions.sh --access=active-seat
systemctl --user restart wayexpand-evdev.service
```

If active-seat/logind is unavailable, explicitly choose the legacy fallback:
```sh
sudo ./scripts/install-evdev-permissions.sh --access=input-group
# Log out and back in if the installer changed group membership.
systemctl --user restart wayexpand-evdev.service
```

### Portal connection issues (libei)

**Symptom:** `wayexpand doctor` shows libei unavailable or keeps reconnecting

**Diagnosis:**
```sh
wayexpand doctor --json | grep -A5 portal_session
```

**Common causes:**
- xdg-desktop-portal not running
- Portal session expired or revoked
- Permission token corrupted

**Fix:**
```sh
# Reset the portal token
wayexpand portal reset

# Restart the daemon
systemctl --user restart wayexpand-input-method.service

# Daemon will re-prompt for permission on next use
```

## Performance Issues

### Slow matches with many snippets

**Expected:** Should be <1ms per trigger with 10,000 snippets

**If experiencing slowness:**
1. Check snippet count: `wayexpand validate --json | grep expansion_count`
2. Run benchmark: `wayexpand doctor --benchmark` (if available)
3. Check for slow commands: `grep -E '^\[\[expansion' ~/.config/wayexpand/*.toml | grep -A5 program`

**Optimize:**
- Remove unused snippets
- Limit command timeouts to 1-2 seconds
- Use specific triggers instead of very short ones

### Daemon memory usage

**Expected:** ~50 MB baseline + 1-5 MB per 1,000 snippets

**Check current usage:**
```sh
systemctl --user status wayexpand-input-method.service
ps aux | grep wayexpand
```

**If using significant memory:**
- Reduce snippet count
- Check for commands with large output (limited to 1 MiB anyway)
- Restart daemon to reset any accumulated state: `systemctl --user restart wayexpand-input-method.service`

## Debugging

### Enable debug logging

View full daemon logs:
```sh
journalctl --user -u wayexpand-input-method.service -f
```

Filter by level:
```sh
journalctl --user -u wayexpand-input-method.service --priority=debug
journalctl --user -u wayexpand-input-method.service --priority=err
```

### Reproduce an issue

1. Gather diagnostic info:
```sh
wayexpand --version
wayexpand doctor --json > /tmp/wayexpand-diagnostic.json
journalctl --user -u wayexpand-input-method.service -n 100 > /tmp/wayexpand-logs.txt
```

2. Describe the steps to reproduce
3. Share the diagnostic info when reporting an issue

### Test with simple config

Create a minimal test config:
```sh
umask 077
cat > /tmp/test-expansions.toml << 'EOF'
[[expansion]]
trigger = ";hello"
replacement = "Hello, World!"
EOF

wayexpand validate /tmp/test-expansions.toml
wayexpand-daemon /tmp/test-expansions.toml  # Test without installing
```

## Getting Help

**Before reporting an issue:**
1. Run `wayexpand doctor` and save the output
2. Check the logs: `journalctl --user -u wayexpand-input-method.service -n 100`
3. Search existing issues: https://github.com/cyberducttape/wayexpand/issues
4. Try the steps above in this guide

**When reporting:**
- Include output from `wayexpand doctor --json`
- Include relevant log excerpts (never include snippet content or passwords)
- Include your desktop/compositor version
- Describe the exact steps to reproduce

**Resources:**
- [GETTING_STARTED.md](GETTING_STARTED.md) — Installation and first steps
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — What's tested vs. experimental
- [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — Enterprise deployment and diagnostics
- [OPERATIONS.md](OPERATIONS.md) — Daemon management and monitoring
- GitHub Issues: https://github.com/cyberducttape/wayexpand/issues

## Troubleshooting Checklist

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > [Troubleshooting](TROUBLESHOOTING.md) > **Checklist**

Use this page when WayExpand is installed but a snippet does not expand. The
flow is intentionally output-driven: collect the diagnostic line, apply the
matching fix, and then retest with a harmless snippet.

## 1. Capture a diagnostic snapshot

Run these commands as the same user who runs the daemon:

```sh
wayexpand --version
wayexpand doctor
wayexpand status --json
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 80 --no-pager
```

Do not paste snippet contents, passwords, portal tokens, or full environment
dumps into an issue. `wayexpand doctor --json` is useful for automation, but
the human-readable output explains the remediation more clearly.

## 2. Read the result from top to bottom

### `Config validation: OK`

Continue to the control socket and backend lines. If it says `FAILED`, or the
file is `not found`/`unreadable`, fix the configuration before investigating
the compositor.

```sh
wayexpand validate ~/.config/wayexpand/expansions.toml
stat -c '%a %U:%G %n' ~/.config/wayexpand/expansions.toml
chmod 600 ~/.config/wayexpand/expansions.toml
```

The file must be a regular file owned by you (or root), and its parent
directories must not be writable by another user. Fix a reported parent with
the exact path printed by `doctor`, for example:

```sh
chmod go-w ~/.config ~/.config/wayexpand
```

Do not use `chmod 777`; it disables the security property the check is
protecting. After editing, run `wayexpand validate` again, then:

```sh
wayexpand reload
wayexpand status --json
```

### `Control socket: disabled` or `control socket unavailable`

The daemon and CLI cannot coordinate without a private runtime directory.

```sh
printf 'XDG_RUNTIME_DIR=%s\n' "$XDG_RUNTIME_DIR"
stat -c '%a %U:%G %n' "$XDG_RUNTIME_DIR"
systemctl --user restart wayexpand-input-method.service
```

If `XDG_RUNTIME_DIR` is empty, start the command from the graphical user
session rather than a root shell, cron job, or unrelated SSH environment. If
you intentionally use `WAYEXPAND_SOCKET`, its parent must exist, be owned by
you or root, and not be group/world-writable.

### Backend line says `Implemented` but no usable combination is printed

`Implemented` means code exists; it does not mean that the current compositor
has advertised the protocol. The probe lines are the decision:

| Doctor output | Meaning | Next action |
| --- | --- | --- |
| `input-method-v2 probe: manager and seat connection succeeded` | Experimental direct Wayland path is available | Use only with explicit opt-in if accepting possible loss of Escape, arrows, or function keys |
| `wlroots probe: virtual keyboard globals available` | A wlroots output path was detected | Use `--source=evdev --backend=wlroots` only when doctor lists it as usable |
| `RequiresPermission` / `permission=Required` | The device or portal needs explicit access | Apply the evdev or portal remediation below; raw evdev is never automatic |
| `Unavailable` / `NotImplemented` | The path cannot be used in this session | Choose another path or compositor |
| `Capture readiness: NOT READY` | No complete source + output pair was found | Resolve the first unavailable probe; do not keep restarting the daemon |
| `Capture readiness: AUTHORIZATION REQUIRED` | A backend such as libei was detected heuristically but requires interactive portal consent; doctor remains unhealthy | Start it deliberately only after reviewing the authorization requirements; treat connection failure as unresolved |
| `Capture readiness: AVAILABLE TO TRY` | A non-invasive protocol or component probe succeeded, but client typing is not end-to-end verified | Start it deliberately only after reviewing the backend limitations; run certification before treating it as supported |

Inspect the conservative selection explanation before choosing a backend:

```sh
wayexpand explain-backend
```

For an explicit input-method-v2 setup, first acknowledge the intentionally
obnoxious warning (experimental; non-text pass-through still requires
compositor/client certification):

```sh
wayexpand setup --mode experimental
wayexpand-daemon --source=input-method ~/.config/wayexpand/expansions.toml
wayexpand doctor
```

For an explicit evdev setup (acknowledges global keyboard visibility and has no
password-field signal):

```sh
sudo ./scripts/install-evdev-permissions.sh --access=active-seat
systemctl --user enable --now wayexpand-evdev.service
wayexpand doctor
```

Evdev has broader keyboard visibility and cannot report password-field focus.
The installer uses active-seat/logind ACLs by default and does not change
permanent group membership. If that mode is unavailable, explicitly choose the
broader legacy fallback with `--access=input-group` (and log out/in if group
membership changes). For libei, the first connection
may ask for portal consent; if a stored consent token is stale:

```sh
wayexpand portal status
wayexpand portal reset
systemctl --user restart wayexpand-evdev.service
```

## 3. Separate service, backend, and snippet failures

### Service is inactive or repeatedly exits

```sh
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 100 --no-pager
```

Interpret the first failure, not the final systemd summary:

- `configuration invalid` → repair the file and run `wayexpand validate`.
- `XDG_RUNTIME_DIR ... required` → launch from the user session or set a
  private runtime environment.
- `input-method ... unavailable` → choose a supported source/backend shown by
  `doctor`; this is not fixed by restarting the same unit.
- `reconnecting` → the compositor or portal disconnected; wait for
  `state=connected`, then try a fresh trigger.

### Service is active and `state=connected`, but a trigger does not match

First prove the config and matcher independently of the compositor:

```sh
wayexpand list
wayexpand search 'hello'
wayexpand test ';;hello'
```

If `wayexpand test` says `no expansion matched`, check the exact trigger,
whether it is disabled, and whether `match_mode = "word-boundary"` is waiting
for a boundary. If the one-shot test matches but typing does not, the remaining
problem is input capture/output or the focused application.

### An app-filtered snippet never matches

Test the unfiltered snippet first. Then check the current support boundary:

- KDE/KWin: run `wayexpand doctor`; window tracking is implemented but still
  best-effort and awaiting broader independent certification.
- Sway, Hyprland, and river: window tracking is not shipped; filters fail
  closed and therefore do not match.
- GNOME/Mutter: no supported window tracker is available; use a global
  expansion or hotkey.

An unavailable tracker should produce no expansion, not an expansion in the
wrong application. See [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) before relying
on filtering for sensitive workflows.

## 4. Final retest

After remediation, verify in this order:

```sh
wayexpand doctor
wayexpand status --json
wayexpand test ';;hello'
```

Then type a simple, non-sensitive trigger in a plain text editor. If it still
fails, report the version, compositor and version, the relevant `doctor`
lines, `status --json`, and a short journal excerpt.

For the longer issue catalog, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
