# Troubleshooting WayExpand

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Troubleshooting**

---

Start with the unified, content-free diagnostic report:

```sh
wayexpand debug
wayexpand debug --json
wayexpand debug --trigger ';http503'
wayexpand debug --test ';hello'
```

For a short live session, explicitly enable sanitized event counters and let
the session expire automatically (60 seconds by default, at most 300 seconds):

```sh
wayexpand debug --live
wayexpand debug --live --seconds 120
```

For app-scoped evdev routes, the live trace reports whether the focused
application is allowed, outside the allowlist, or unknown. It also reports
sanitized input, matcher, and output counter changes; it never prints
application IDs, triggers, replacement text, or clipboard contents. An
`unknown` scope means the compositor focus tracker did not provide an app ID,
so matching remains disabled for that focus until identity is available.

When scope is `outside_allowlist`, focus the affected terminal and run
`wayexpand debug --focused-app-id`. This explicit diagnostic prints only the
bounded application ID or, when KWin cannot provide one, the authenticated
focused process's exact executable path if the host permits that lookup (never
the window title or typed text), and the daemon's current app-scope decision.
When it succeeds, JSON reports `focused_app_id: null` and
`focused_process_executable`; use the path only in an explicit
`process_executable_exact:` allowlist entry. If the host denies executable
inspection but permits reading `/proc/<pid>/comm`, the command reports a
`focused_process_name` separately. That kernel task name is mutable and is a
lower-assurance identity; matching requires an explicit
`process_name_exact:<name>` filter and is never inferred from an application
ID or executable-path filter. Do not use it when policy requires a stable
executable identity. If neither identity is available, the command reports
that limitation and no identity. Its JSON
`matching_status` is one of `allowed`,
`suppressed_unknown_focus`, `suppressed_outside_allowlist`,
`scope_unverified`, `unrestricted`, or `unknown`. If it reports
`outside_allowlist`, automatic matching is suppressed for the reported focus;
compare the ID with `settings.terminal_apps` and `settings.raw_capture_apps`.
`suppressed_unknown_focus` means terminal-scoped matching is refused because
the tracker supplied no safe app ID. Check `wayexpand status --json` for
window-tracker health, then use `wayexpand debug --live` to see whether scope
becomes available. The command reports `unavailable` if the tracker has no
safe application ID or policy withholds focus identity. If the running daemon
reports an unsupported command, update and restart it before retrying.

`wayexpand debug --json` reports `WX-EVDEV-FOCUS-001` when terminal-only
matching is suppressed because focus identity is unknown, and
`WX-EVDEV-FOCUS-002` when the focused app is outside the explicit allowlist.
`WX-EVDEV-FOCUS-003` means KWin supplied a process ID but the host denied
WayExpand permission to verify its executable. In that case, adding an exact
executable-path filter cannot help until the host provides a trustworthy
identity. Do not weaken `/proc` permissions or use a title/wildcard; use an
application-ID-capable route or the manual copy-and-paste picker. The finding
contains no application ID, executable path, or window title.
`WX-EVDEV-FOCUS-004` means the focused process is outside the allowlist and
only a mutable process name is available. It explains the lower-assurance
`process_name_exact:` option; this is opt-in and should not be used to satisfy
a policy that requires stable application identity.
`WX-EVDEV-FOCUS-006` means KWin reported its own PID for a compositor-owned
surface, not a client process. Focus the actual application and rerun the
diagnostic. Never allowlist `kwin_wayland` to make this state appear supported.

Live output reports route-state changes, aggregate input-event counts, IBus
activity counters, and output transaction IDs/states. It never records key
values, triggers, replacement text, or clipboard contents. JSON live output is
newline-delimited event records. A debug report does not certify insertion in
an application; test a harmless expansion in each application class you rely
on. A successful GTK test does not certify Qt, Electron, terminal, browser,
remote-desktop, or sandboxed-app behavior.

`debug --test TRIGGER` is an interactive desktop integration check for an
enabled, static, non-command snippet. Review the expected text first with
`wayexpand preview`, select the target application class, and confirm before
typing the trigger. It waits for sanitized capture/output evidence and then
asks you to confirm exact visible text. It never presses Enter, never retries
an uncertain output, and does not claim coverage beyond the one test session.
The GUI's **Test Expansion → Desktop Test** runs the same observation in a
disposable field after explicit consent, then displays the expected text for a
separate user confirmation. Activity counters are not correlated to the test
trigger, so the GUI labels them as observed activity and never treats them as
proof of insertion by themselves.

Finding IDs such as `WX-CONFIG-001`, `WX-GNOME-IM-001`, `WX-IBUS-002`,
`WX-IBUS-RELOAD-001`, `WX-INPUT-001`, `WX-OUTPUT-001`, `WX-POLICY-001`,
`WX-SERVICE-002`, and `WX-VERSION-001` are stable references
for support reports and automation.

### Browser text fields do not expand

Browser and Electron clients are not certified as a general WayExpand route.
On Plasma, native Wayland browsers use KWin's IBus Wayland bridge. Do not set
`GTK_IM_MODULE=ibus` or `QT_IM_MODULE=ibus` globally in a Plasma Wayland
session; those legacy toolkit overrides can conflict with KWin's compositor-
managed IBus integration. Check
`wayexpand ibus status --json` for the `desktop` check. If it reports
`WX-KWIN-IBUS-001`, select **IBus Wayland** in System Settings → Keyboard →
Virtual Keyboard, then log out of Plasma and back in. If KWin repeatedly
crashes `ibus-ui-gtk3`, update or repair the system IBus package and inspect
`coredumpctl info ibus-ui-gtk3` before retesting.

If the diagnostic reports `WX-KWIN-IBUS-002`, it found a recent IBus helper
segmentation fault for the current user. The finding includes the number of
matching crashes in its bounded seven-day sample and the latest crash age when
the journal provides a timestamp. When available, it also reports the package
name and version recorded for the process that crashed. This is the crashed
process's package metadata, not proof of the currently installed version or
that a distribution fix is present. When
the latest dump has both `g_utf8_pointer_to_offset` and Wayland-dispatch
frames, the diagnostic reports that it resembles Fedora bug 2444787, a
Wayland surrounding-text anchor crash. This signature is evidence about the
IBus helper, not proof that every crash has the same cause. Inspect the
recorded PID with `coredumpctl info <PID>` and check whether your
distribution's IBus package includes the corresponding fix. If the helper is
running again, first run
`wayexpand ibus watch --seconds 30` and test a harmless trigger in the affected
application; a historical crash alone is not a reason to restart the desktop
session. If input still does not arrive, update the IBus package supported by
your distribution and log out of Plasma and back in. If the helper is not
running, inspect the coredump, update the IBus package, select **IBus Wayland**
in Virtual Keyboard settings, and log out and back in. If the crash recurs,
report the backtrace and exact package version to your distribution or IBus
maintainers.

If `wayexpand ibus status` reports `WX-IBUS-TOOLKIT-001`, remove the legacy
variables from Plasma login files, run
`systemctl --user unset-environment GTK_IM_MODULE QT_IM_MODULE`, then log out
and back in so the session bus and compositor start with clean environment.

If `wayexpand debug --json` reports `WX-IBUS-004`, the IBus daemon process is
not answering on its private D-Bus. A running process or a remembered engine
selection does not establish that the desktop can deliver browser input to
WayExpand. Restore the desktop-managed IBus session, then confirm the daemon
check passes in `wayexpand ibus status --json` before testing a browser field.

An older WayExpand CLI may instead say that `org.freedesktop.IBus` has no
owner on the desktop session bus. IBus normally serves clients on the private
address returned by `ibus address`, so that message alone does not show that
the daemon is down. Update WayExpand and rerun `wayexpand ibus status --json`;
the current diagnostic probes IBus's private bus and checks KWin's Wayland
bridge separately. Do not restart IBus solely because the old session-bus
check failed.

Check the capture scope with `wayexpand status --json`. If `source` is `evdev`
and `terminal_apps_only` is `true`, the daemon matches only its configured app
allowlist. For the terminal-only setup that means terminal IDs; the explicit
browser fallback may add `raw_capture_apps` to the same allowlist. `wayexpand
ibus status` reports the IBus route separately. Do not broaden capture to
every application to work around an inactive IBus path.

To observe whether the browser sends input to IBus, run
`wayexpand ibus watch --seconds 30`, switch to the browser, and type harmless
text. The watch reports counts and field capabilities only, not the typed text.
If it reports no new keys, the browser is not routing this field through the
WayExpand IBus engine. If it reports `no-surrounding-text`, IBus cannot safely
replace a trigger in that field. The telemetry does not identify the app, so
do not treat this result as proof about every browser field. Do not add a
browser to `terminal_apps`. If you explicitly accept global raw keyboard
observation and the absence of password-field detection, configure
`settings.raw_capture_apps` and follow the browser-scoped IBus/evdev setup in
`docs/IBUS_ENGINE.md`. IBus yields to evdev only when authenticated compositor
focus identifies one of those apps; both routes fail closed when focus is
unknown. If no app list exists, the explicit setup seeds known browser app IDs;
replace them if your browser uses a different ID. This remains uncertified and
may use non-atomic replacement. Otherwise
use the Quick Picker's reviewed copy-and-paste path.

### WX-POLICY-001

IBus is active, but replacement is disabled because `compatibility_mode` is
`"off"`. This is the default because IBus trigger deletion and replacement
commit are separate operations. If you choose to accept that non-atomic route,
run `wayexpand setup --backend ibus` interactively and review its replacement
authorization prompt. Do not use `--yes` to grant consent. Managed systems
should change this setting only through an approved policy. Then rerun
`wayexpand ibus status` and test a harmless expansion in the intended app.

### WX-GNOME-IM-001

The GNOME IBus session, source registration, or active source could not be
verified. Active-source status comes from the live GNOME Shell
`InputSourceManager`; the deprecated `org.gnome.desktop.input-sources current`
key is ignored and does not prove activation. If the active-source check is
unknown, update/enable the WayExpand GNOME Shell extension and log out and back
in so its Shell bridge is loaded.
Inspect the individual GNOME session, source-registration, and active-source
checks with `wayexpand ibus status --json`. Run
`wayexpand repair --gnome-input-source` to register and select the source
through GNOME Shell, then independently verify the IBus engine.
Configuration readiness does not prove that a text field receives input or
that an application applied a replacement; use `wayexpand debug --test
';;hello'` and confirm the visible result.

### GNOME setup continuation

If setup installed the Shell extension and asked for a logout/login, its
continuation is valid only in a GNOME Wayland session and expires after 14
days. On another desktop, ordinary `wayexpand setup` uses that desktop's
normal backend recommendation and leaves the continuation untouched. To
discard an abandoned or unsupported continuation, run
`wayexpand setup --cancel-resume`.

### WX-PORTAL-001

**Portal authorization required.**

WayExpand has not established the requested RemoteDesktop/EIS session. Review
the portal prompt and approve only when you intentionally started a libei test
or selected a route that needs it. To request a fresh session, run
`wayexpand backend test --injection`; to clear a saved restoration token, run
`wayexpand portal reset`. This permission does not grant evdev access.

### WX-INPUT-001

**Input route unavailable.**

The active route cannot currently provide usable matching input. Run
`wayexpand debug`, `wayexpand backend profile --current`, and
`wayexpand setup --check --json`. Review the reported live capabilities and
policy separately from the desktop profile; a profile is guidance, not proof
that a protocol is available or certified.

### WX-OUTPUT-001

**Output route unavailable.**

Inspect `wayexpand status` and `wayexpand debug`. If an earlier insertion is
reported partial or uncertain, inspect the target application's text before
retrying. Never replay an output whose effects are uncertain.

For detailed backend and service snapshots:

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

**Diagnosis:** start with `wayexpand explain`, which asks the running daemon
why a specific trigger would or would not expand right now: whether the trigger
or an alias is recognized, the match mode, the focused application against the
snippet's app filter, pause, password-field and composition state, organization
policy, and whether the output backend is connected. It ends with the first
check that blocks the expansion. (Where administrator policy makes the control
socket read-only, live answers give only a generic reason; use `--offline` for
detailed configuration checks.)

```sh
wayexpand explain ';sig'           # Live answer from the daemon
wayexpand explain ';sig' --json    # The same, for scripts
wayexpand explain ';sig' --app org.kde.konsole --offline  # From the config alone
```

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

### Form snippet is recognized but no form opens

Form fields such as `{{field:hostname}}` need the route to verify the trigger
at the text cursor and identify the same input target after the form closes.
The IBus route uses the IBus input-context identity and checks surrounding
text; its replacement is not atomic and the route is not production certified.
The evdev/libei route cannot verify the trigger at the cursor, so it refuses
forms even when ordinary static snippets work.

```sh
wayexpand status --json
wayexpand explain '.incident' --json
```

Check `replacement_assurance` in status and the `replacement assurance` check
in the explanation. On a best-effort route, remove `{{field:...}}` to make the
snippet static, or switch to IBus if it is available and its limitations are
acceptable. Do not disable replacement checks to make a form open.

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

### Output route stalled

**Situation:** `wayexpand doctor` prints `Output route: stalled`, or
`wayexpand status` shows `output_health=stalled`; nothing expands.

**Why this happens:** an output operation did not finish within its deadline
and the backend is still inside a compositor or portal call. WayExpand will
not open a new output connection until that call returns, because a late
operation could otherwise type into whichever window is now focused. Some
backends never return from a wedged call.

**What to do:**
1. Restart the daemon; this is the controlled reset and discards the stuck
   call. `doctor` names the unit, for example:
```sh
systemctl --user restart wayexpand-evdev.service
```
2. Check the text you were editing. `output_uncertain_total` counts
   transactions whose target effect is unknown; they are never retried.
   `output_partial_total` specifically counts failures after insertion began.
   The GUI raises a separate partial-insertion warning, and the recent outcome
   trace identifies the transaction without retaining its text.
3. If it recurs, capture `journalctl --user -u <unit>` around the stall for a
   bug report.

## Compositor-Specific Issues

### GNOME Shell extension readiness

`wayexpand doctor --gnome --json` reports extension readiness independently
from the extension files on disk:

- `not-loaded-this-session` means GNOME answered the status query, the enabled
  extension matches the installed build, and Shell has not loaded it this
  session. Log out and back in, then rerun the doctor.
- `session-bus-unavailable` means the Shell status query could not establish a
  usable session-bus connection. Run the doctor from the active GNOME session
  and inspect the D-Bus/extension evidence; this state alone is not a reason to
  log out.
- `extension-failed` means GNOME reported a load error or the tracker protocol
  failed. Inspect the reported extension error and handshake before reinstalling
  or restarting anything.
- `unknown` means there is not enough evidence to choose one of those causes.
  Follow the targeted command in the report; do not assume a desktop restart is
  required.

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

**Symptom:** When enabling app filters, a 1–2 second delay may appear while the
GUI waits for KWin to identify the focused application.

**Why it happens:** The `loadScript()` call returns before the D-Bus object is fully registered

**This is expected and no longer blocks the window.** Detection runs in a
background worker; the GUI shows a spinner and offers **Cancel** while the
first KWin script registration completes. Subsequent calls are usually faster.

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
ls "$XDG_RUNTIME_DIR"/wayexpand-window-tracker-*.js 2>/dev/null || echo "None found"
# Stop the WayExpand service before removing stale tracker scripts.
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

Matching itself measured about 3 µs per input event at 10,000 snippets
([BENCHMARKS.md](BENCHMARKS.md)). Visible delays usually come from output
pacing (the libei keysym fallback types about one character per 12 ms) or
from command-backed snippets.

**If experiencing slowness:**
1. Check snippet count: `wayexpand validate --json` (`expansion_count`)
2. Compare matcher and injection latency: `wayexpand status --json`
   (`matcher_latency_p95_us`, `injection_latency_p95_us`, and
   `injection_latency_profiles` by output mode and size)
3. Check the output mode: `wayexpand doctor` reports whether text is
   inserted directly or typed key by key
4. Check for slow commands: `grep -E '^\[\[expansion' ~/.config/wayexpand/*.toml | grep -A5 program`

**Optimize:**
- Remove unused snippets
- Limit command timeouts to 1-2 seconds
- Use specific triggers instead of very short ones

### Daemon memory usage

The daemon reports its own resident memory in `wayexpand status --json`
(`memory_rss_kib` and `memory_high_water_kib`). The 10,000-snippet benchmark
process peaked at 125,304 KiB (about 122 MiB) RSS, including the benchmark harness; see
[BENCHMARKS.md](BENCHMARKS.md).

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
- GNOME/Mutter: the WayExpand Shell extension provides focus tracking to the
  IBus service, but this path has no accepted physical-session certification.
  If tracking is unavailable or disconnected, app-filtered snippets fail
  closed; use a global expansion or hotkey until the target path is verified.

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
