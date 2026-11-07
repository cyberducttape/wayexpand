# IBus engine integration

`wayexpand-backend-ibus` contains the native-engine boundary used by an IBus
service. It feeds IBus keysyms into the same `ExpansionEngine` used by the
Wayland daemon and returns protocol-neutral actions:

- ordinary printable keys are committed immediately, so they are not delivered
  twice by the toolkit;
- a match emits `DeleteSurroundingText` followed by `CommitText`. These are
  two separate signals, so the replacement is **not atomic**: a failure
  between them can leave the trigger deleted without its replacement. The
  route therefore reports `atomic_replace = false`;
- the deletion covers only text the client actually holds: the final key of
  an immediate trigger is consumed and never delivered, so it is not deleted;
- a word-boundary delimiter is included in the replacement commit;
- nothing is deleted unless the client's reported surrounding text confirms
  that the trigger sits immediately before a collapsed cursor (see below);
- shortcuts, navigation keys, backspace, and disabled focus are forwarded to
  the client and clear matcher state.

The `wayexpand-ibus` binary now supplies that service wrapper. It claims a
private IBus engine name, exposes the standard factory/engine objects, and
translates actions into `CommitText`/`DeleteSurroundingText` signals. The
adapter remains separate so matching and deletion semantics stay testable
without an IBus session.

`app_filter` entries fail closed in this mode because IBus does not provide a
portable focused-window identity.

## Sensitive fields

Capture is off until a field positively reports a known, non-sensitive
content type. IBus delivers it by setting the engine's write-only
`ContentType` property (`(uu)`: purpose and hints) through
`org.freedesktop.DBus.Properties.Set`; there is no `SetContentType` method.
Only a content type IBus has set can enable matching.

A new engine has no content type, so capture starts disabled. `FocusOut` and
`Disable` disable capture. ibus-daemon remembers the content type it last set
on each engine and writes the property again only when a newly focused field's
type differs, so `FocusIn` clears matcher state and then restores the engine's
last content type: a refocused password field stays disabled, and a refocused
ordinary field resumes matching. `Reset` does not change the field's content
type. The following are treated as sensitive:

- purposes `PASSWORD` (8) and `PIN` (9);
- any purpose newer than this build knows (greater than `DATETIME`, 13);
- the `PRIVATE` and `HIDDEN_TEXT` input hints, on any purpose.

A client that never sends a content type therefore gets no expansion.

## Surrounding-text verification

The engine requests surrounding text (`RequireSurroundingText`) when a field
gains focus and tracks the client's reported text, cursor, and selection
anchor, advancing that model over the edits it emits itself. A replacement is
emitted only when that text ends with the trigger characters the client holds
and there is no selection. Otherwise the replacement is refused before
anything is deleted and the key is delivered normally. `Reset`, focus changes,
and keys the client applies itself drop the model until the client reports
its text again. Clients without `IBUS_CAP_SURROUNDING_TEXT` get no
replacements.

The IBus service loads the same root-owned `/etc/wayexpand/policy.toml` as the
daemon. Invalid or insecure policy files prevent startup, and active policy
limits (including replacement size and allowed backend) are enforced for IBus
expansions, and capability requirements are checked against the IBus route's
profile: in safe mode, `require_atomic_replace` disables IBus expansion
because the route is not atomic, while `require_sensitive_focus` is satisfied
by content-type reporting. Direct executable commands remain disabled in IBus because the
IBus service is not the hardened `wayexpand.service` command-execution
boundary. Named Action Broker commands are allowed: they cross the broker's
authenticated Unix-socket boundary and fail closed when the broker is
unavailable. The backend is identified as `ibus` for `allowed_backends`
policy checks, separately from `input-method-v2`; a policy that permits both
must list both.

Each IBus `CreateEngine` request owns one engine object and adapter. IBus
releases that object through `Destroy`; the service removes the adapter from
its registry and unregisters the D-Bus object at that point.

Named actions run on the engine's command workers so `ProcessKeyEvent` never
waits for them. When a result is ready the worker wakes a single completion
thread with the owning engine's path; there is no polling. That thread locks
the engine's adapter, discards results that are stale (any key, reset, or
focus change since the action was queued), checks policy and surrounding
text, emits the delete and commit signals, and records the expansion as
applied only after both were emitted.

Key events are converted with xkbcommon's keysym-to-Unicode mapping, so
legacy keysyms (for example `Greek_alpha` or `Cyrillic_a`) and keypad digits
match like explicit Unicode keysyms. Dead keys and Compose are resolved by the
toolkit before IBus sees the result. These paths are unit tested; behaviour
with real clients across layouts is not yet certified.

## Setup

Run:

```sh
wayexpand setup                 # recommended route, selected from detected capabilities
wayexpand setup --backend ibus  # explicitly choose IBus
```

Setup works in phases and reads each result back rather than trusting exit
codes. **Preflight** checks everything it needs (IBus, the engine binary,
the configuration, KDE's IBus Wayland launcher and `kwriteconfig6`, the
services, and for `--terminals` the evdev unit and consent marker) and stops
with the exact fix before changing anything. **Prepare** makes only inert
changes. **Activate** records the session first (the active input method,
IBus' input-method list, KWin's Virtual Keyboard, and which WayExpand
services ran), then changes it. **Verify** runs the same checks as
`wayexpand ibus status`; if activation or verification fails, setup restores
the recorded session and says so. The steps are:

1. Checks whether the **running** ibus-daemon already reads the component
   manifest (its `IBUS_COMPONENT_PATH`, otherwise the distribution directory).
   Distribution packages own and install this file themselves. A user/tarball
   install displays a plan before requesting one-time desktop authorization to
   register the manifest when the running daemon's component directory is not
   writable by the user. An explicitly configured writable
   `IBUS_COMPONENT_PATH` can be used without elevation; WayExpand does not
   assume the ordinary XDG data directory is read by IBus. This does not
   install application files system-wide or grant keyboard-device access. The
   shared manifest points to the root-owned
   `/usr/local/libexec/wayexpand/wayexpand-ibus` executable, never to the
   installing user's home directory. Setup stages the current engine there,
   verifies ownership and permissions, and journals it with the manifest so
   failed activation can restore both. Later users reuse the same system
   registration rather than replacing it with a home-specific path. The
   IBus diagnostic reports `WX-IBUS-MULTIUSER-001` if an older shared
   manifest still resolves its executable into one user's home; repair that
   registration with `wayexpand setup --backend ibus --install-system-integration`.
   The GUI has a separate checkbox for that
   authorization. In scripts, `--yes` alone deliberately skips the privileged
   step; pass `--install-system-integration` only when that change is intended.
   If authorization is unavailable, install the distribution package, restore
   the desktop `pkexec` authorization path, or select another supported input
   route. No ordinary per-user IBus component directory is assumed to work.
   Setup removes its private staged XML on success and failure. After resolving
   the authorization problem, the supported retry is
   `wayexpand setup --backend ibus --install-system-integration`.
2. On KDE Plasma Wayland, sets KWin's Virtual Keyboard to IBus Wayland (the
   same as System Settings → Keyboard → Virtual Keyboard). Without it,
   Wayland applications never reach IBus. A hand-started ibus-daemon is
   replaced, because it has no Wayland input-method connection.
3. Restarts IBus only if its registry does not list the engine yet (IBus
   loads components only at startup, so input methods pause briefly), adds
   `wayexpand` to IBus' input methods, and selects it.
4. Asks for `compatibility_mode` consent if it is `off`: IBus replacement is
   verified but not atomic, and `off` refuses every replacement. `--yes`
   never grants this consent.
5. Stops WayExpand daemons that capture keys in every application, since they
   would expand snippets a second time, and starts the core companion service:
   `wayexpand-control.service` (control socket, quick insert and hotkeys; it
   captures no keys). Optional terminal coverage uses a separate evdev daemon
   restricted to `terminal_apps`; setup offers it only after the user confirms
   a real IBus insertion. It is never enabled by `--yes`, and terminal setup
   still requires the separate raw-capture consent marker. Existing terminal
   coverage is kept when setup runs again.

`wayexpand ibus status --json` (IBus-route schema 2) separates configuration readiness from
operational readiness. On GNOME it independently checks the GNOME Wayland
session (including the live Shell bus name), source registration, the active
source reported by GNOME Shell's `InputSourceManager`, and content-free input
activity from a normal field. GNOME ignores the deprecated
`org.gnome.desktop.input-sources current` setting; it is never used as
activation or readiness evidence. The WayExpand GNOME Shell extension uses Shell's own source
activation API and verifies the resulting live source; the IBus engine is
checked independently. Missing extension/API evidence is `unknown`, never a
pass. `mru-sources` is persistence context only and is not used to infer or
trigger activation. Seeing key events or emitted replacement actions
does not prove that the target application applied the edit: IBus does not
receive an acknowledgement of the resulting document text. `ready` therefore
remains false until target insertion is independently verified. Use
`wayexpand debug --test ';;hello'` and confirm the result in the application.
`wayexpand repair --gnome-input-source` repairs the GNOME source list, asks
GNOME Shell's live input-source manager to select WayExpand, reads that
manager's selection back, then verifies the IBus engine independently; it
does not claim that a real field insertion succeeded. If the Shell bridge is
not active or current, setup stops and explains that the extension must be
enabled or the session restarted. The Diagnostics window shows the same
separate evidence.

The GNOME extension also exposes a build identity from the metadata loaded by
the running Shell process. `wayexpand doctor --json` reports the bundled,
on-disk, and live identities. If the installed files are current but Shell is
still running an older identity (or cannot report one), diagnostics say
`logout-required`; logging out and back in loads the installed extension. The
`x-wayexpand-build-id` metadata value is the SHA-256 of `extension.js`; the
backend test verifies the pair, so extension code changes must refresh that
metadata identity before packaging.

## Diagnostics

On GNOME Wayland, `wayexpand setup` performs component registration,
replacement-consent handling, GNOME input-source registration and activation,
then offers the harmless `;;hello` real-application test. Existing GNOME
keyboard sources are retained and the engine uses the system keyboard layout.
The command ends with one explicit state: `WORKING` after the user confirms an
exact insertion in the tested application, `CONFIGURED, AWAITING TEST` when
the interactive test was deferred, or `ACTION REQUIRED` when a prerequisite
or attempted test fails. Component registration and engine activation alone
are never reported as a working text expander. The test does not certify
other applications or session persistence.

If IBus is configured but a required route check fails, setup returns a
nonzero result with `SETUP RESULT: ACTION REQUIRED` and a stable finding ID;
the valid configuration is retained for diagnosis. A second unrestricted
capture route beside IBus is also an action-required condition because it can
expand the same trigger twice. Run `wayexpand help finding <WX-ID>` for the
repair and retest steps. A route with no hard failure but no user-confirmed
application insertion is reported as configured and awaiting test.

```sh
wayexpand ibus status   # every link of the route, with the fix for each failure
wayexpand ibus watch    # switch to an app and type: explains each field
```

The JSON checks expose stable problem IDs for runtime evidence too:
`WX-IBUS-003` means no normal-field input was observed, `WX-IBUS-005` means
the engine recorded an output failure (the target may contain partial text),
`WX-IBUS-006` means replacement actions were emitted but the resulting text
cannot be verified by IBus, and `WX-IBUS-007` means no replacement was
observed in the current engine session. These distinguish capture activity
from target-side success; never automatically replay an uncertain insertion.
Use `wayexpand help finding <WX-ID>` for a next step.

`wayexpand doctor --gnome` presents three diagnostic sections: IBus input
capture, snippet insertion (including replacement consent, output route, and
the distinction between emitted actions and a verified target result), and
GNOME Shell window tracking. The window section separately checks extension
installation, Shell load state, live build identity, the tracker D-Bus
handshake, and the daemon's current focus identity. Focus identifiers and
titles are not included in the report. Its versioned JSON schema is 3 and
includes a stable `error_id`, sanitized `evidence`, `remediation_command`,
remediation detail, and `retest_instructions` for every non-passing stage. It
also compares the running daemon's build identity and control-status schema
with the CLI. Use
`wayexpand debug --gnome --interactive` for a harmless, user-confirmed test
in a real application; activity counters are explicitly uncorrelated with
the typed test trigger, and the test does not certify other applications or
logout persistence. `wayexpand support --gnome` writes the ordinary redacted
support report plus sanitized GNOME/IBus source, component-location class,
service, policy, test-state, and stable error-ID fields. It excludes source
tuples other than WayExpand presence/active state, user paths, keystrokes,
snippet text, clipboard data, and environment values. The older
`wayexpand support-bundle` command remains available. The engine publishes
content-free activity (counts and states only, never text) to
`$XDG_RUNTIME_DIR/wayexpand-ibus-status.json`; field verdicts distinguish
`expanded`, `ready`, `sensitive` (password/PIN), `no-surrounding-text`
(typical of terminals), `text-mismatch`, `refused-by-policy`, and
`no-content-type`.

## Terminals

Terminals do not report the text before the cursor, so IBus cannot replace a
trigger there (the engine refuses rather than deleting blindly). After a
successful real IBus insertion test, setup can offer optional terminal
coverage (or use `wayexpand setup --backend ibus --terminals` to request it
after completing the same insertion test). It uses a separate evdev daemon
with `WAYEXPAND_TERMINAL_APPS_ONLY=1`
and expands only while one of the configured `settings.terminal_apps` is
focused (KWin window tracking; fail-closed otherwise). IBus keeps handling
other applications, so nothing expands twice. evdev reads raw keyboard input
including password fields, so this requires the separate consent marker
described in [BACKENDS_SENSITIVE_FIELDS.md](BACKENDS_SENSITIVE_FIELDS.md).

## Protocol notes

- IBus sets an engine's content type through the write-only `ContentType`
  property (`Properties.Set`), and only when it differs from the value it last
  set on that engine; the engine keeps the last content type across refocus.
- ibus-daemon launches the component's `exec` and waits for the component
  `<name>` (`org.freedesktop.IBus.WayExpand`) to appear on its bus before it
  calls `CreateEngine`; the engine owns exactly that name.
- `ibus engine wayexpand` exits 1 even after a successful switch, because the
  engine keeps the user's layout (`<layout>default</layout>`).

`scripts/test-ibus-e2e.sh` exercises these against a private, real
ibus-daemon (also run in CI).
