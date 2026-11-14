# Backend plan

The core exposes two independent contracts:

```text
InputSource -> InputEvent -> ExpansionEngine -> TextInjector
```

The input-method-v2 contract is experimental; see the [protocol
definition](https://wayland.app/protocols/input-method-unstable-v2). The
portal-backed libei path follows the
[RemoteDesktop ConnectToEIS contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html).

The first production backend should be selected as a complete pair, not as an
isolated injector. A text expander needs both a source of input and a safe way
to erase and insert text.

Route planning keeps installation, compositor support, authorization,
connection, organization-policy permission, and certification as separate
assessment fields. A route can be a useful setup candidate while still being
unsupported, unauthorized, or unconnected in the current session. In
particular, KDE/GNOME desktop-name detection is only a portal-provider hint;
it does not establish that the portal interface exists, that consent was
granted, that EIS connected, or that injection was negotiated or verified.
Use `wayexpand setup --check --json` or `wayexpand doctor --json` to inspect
the normalized assessment.

Desktop guidance is also available in the checked-in integration profiles:
`wayexpand backend profile` lists profiles and `wayexpand backend profile
--current --json` combines the detected desktop profile with a separate live
capability- and policy-based route recommendation. A profile's catalog route
is guidance only. Its verified-applications list stays empty until accepted
real-session certification evidence exists.

For an explicit one-shot check of the two evdev+libei halves, run:

```sh
wayexpand backend test --capture
wayexpand backend test --injection
```

The capture check requires a confirmation, opens readable keyboard devices
for at most 15 seconds, and reports only whether sanitized activity arrived.
It does not print or store key values; physical typing still reaches the
focused application, so use a disposable text field. The injection check
connects and negotiates an EIS session, then inserts a fixed harmless marker
only after confirmation. It sends no Enter key, never retries an uncertain
write, and counts as target-verified only after the operator confirms the
marker appeared once. The one-shot check does not persist a new portal
restoration token. A passing capture check and a passing injection check
are separate evidence; neither certifies the combined route or other apps.

The route catalog reports three separate facts: `trigger_precondition` says
what is checked before erasing, `output_transaction` says whether erasure and
insertion are one protocol batch or separate operations, and
`application_result` says whether the backend can observe the resulting
document. Current routes report `unobservable`; an accepted backend request
does not prove that the target application applied the expected text. IBus
checks surrounding text against the trigger, but emits deletion and commit as
separate signals, so that precondition does not make its replacement atomic.
Input-method-v2 submits deletion and commit in one protocol batch, but still
cannot observe the target application's final document.
`wayexpand backend list --json` schema 2 exposes these as separate fields.

Keystroke injectors (libei, wlroots virtual keyboard) are wrapped by the
daemon's assurance gate when they connect: they erase only with the user's
`compatibility_mode` consent and a focus check immediately before typing, and
a refusal leaves the text untouched. See "Insertion guarantees" in
[SECURITY.md](../SECURITY.md#insertion-guarantees).

## libei and the RemoteDesktop portal

This is the preferred cross-desktop direction. The portal creates a user-
approved remote desktop session, and `ConnectToEIS()` returns the file
descriptor used to establish the libei connection. The implementation must
handle consent denial, reconnect, compositor restart, and session revocation.

WayExpand now has an isolated `wayexpand-backend-libei` output backend. It
uses the pure-Rust `reis` implementation and can use either `LIBEI_SOCKET` or
the XDG RemoteDesktop portal; select it explicitly with `--backend=libei`. It
requests both `ei_text` for UTF-8 insertion and `ei_keyboard` for Backspace
erasure. Long replacements are split at Unicode boundaries to respect the
protocol's per-request text limit. When the EIS server resumes a keyboard
device without `ei_text` (observed with xdg-desktop-portal-kde on KWin 6.6),
the backend falls back to synthesizing individual key presses over
`ei_keyboard` using the keymap the server itself supplies. That fallback is
layout-dependent -- only characters the server's keymap can produce on the
active layout, unmodified or with that keymap's own Shift, AltGr (ISO Level3)
and Level5 keys, are typeable (so `@` is typed as AltGr+Q on a German
layout). Before every erase or insertion the backend applies the server's
`ei_keyboard.modifiers` reports: a layout switch or Caps Lock change rebuilds
the character map, a paused or removed device (a keymap change replaces the
device) refuses and reconnects. A keymap with several layouts uses the
server-reported group or the daemon's compositor layout provider. Provider
data is accepted only after compiling its XKB rules/model/layout/variant/
options and checking that its ordered layout groups match the EIS keymap.
The EIS keymap remains authoritative for actual key synthesis; requiring all
non-text keysyms to match would reject otherwise compatible maps (for example,
when media-key definitions differ). Missing, reordered, or ambiguous layout
state is refused rather than guessed. KWin's provider reads its
keyboard-layout service; Hyprland uses compositor-owned keyboard data; GNOME
uses the Shell extension's live `currentSource.xkbId` plus the model and
options from the exact settings source GNOME's InputSourceManager applies
(session GSettings or systemd-localed); GNOME 45's session settings lack a
model field, so the provider uses Mutter's `pc105` default for that case.
GNOME evdev matching remains disabled if the Shell extension cannot provide
that active state or XKB cannot compile the reported map. These paths still
require live certification. A
replacement
containing an unreachable character is rejected with an error before
anything is typed, rather than partially or incorrectly inserted. Portal use
is never automatic: selecting this backend may request desktop-control
consent and the portal session is retained for the injector's lifetime. A
restoration token is stored with user-only permissions after successful portal
authorization using an ownership-checked, no-follow, durable atomic write, so
normal reconnects can restore consent without another dialog. If the token
expires or is revoked, fresh consent is required; use `wayexpand portal reset`
to forget the local token.
The backend also caps direct text submissions at 1 MiB and validates them before
queuing erase events.
Handshake and initial device discovery use explicit bounded polling; an
unresponsive EIS endpoint cannot hold daemon startup indefinitely.

### Optional Fcitx5 direct commit

Set `settings.fcitx5_direct_commit = true` to let the libei injector use a
compatible Fcitx5 D-Bus bridge when one is already installed. The bridge
verifies the exact UTF-8 suffix at the input context cursor and performs the
delete/commit through Fcitx5; when surrounding text is unavailable it may use
only its short-lived, input-context-bound fallback permit. WayExpand does not
start, install, or compile the bridge, and silently keeps the ordinary libei
route when no bridge is present.

Bridge refusals for selections, mismatches, password fields, sensitive hints,
unknown statuses, and transport failures are fail-closed. In particular,
WayExpand never follows an uncertain D-Bus result with raw backspaces. The
option is disabled by default and does not make libei claim atomic replacement
or sensitive-field awareness. When the EIS device only exposes its
layout-dependent keysym fallback, the bridge is tried before requiring an
active EIS layout group. Thus a verified Fcitx5 Unicode commit can still work
when KWin omits that group. If the bridge reports that no Fcitx input context
is focused, the ordinary fallback's layout, representability and pacing
checks are applied before any raw erase. A missing group never selects the
first configured layout.

**Character insertion latency (ei_keyboard fallback):**
When the EIS server provides only `ei_keyboard` (no `ei_text`), the backend
synthesizes individual key presses for each character. To ensure reliability
and prevent key merging at the compositor, a safe delay is added between each
synthesized character. This means:

- **Normal case** (ei_text available): Sub-millisecond insertion (fast)
- **Fallback case** (ei_keyboard only): about 12ms per character (slower)

For short snippets (signatures, small templates), this is imperceptible.
For long replacements (multi-paragraph code blocks, long administrative text),
the total insertion time becomes noticeable:

- An 8-character replacement payload alone is about 97ms at 83 events/sec;
  trigger erasure, delimiters, and cursor movement add to the transaction time.
- 100-character replacement payload: ~1.2 seconds, plus trigger erasure and any
  delimiter or cursor work

Longer replacements are rejected before any erase or insertion. The configured
character ceiling is `[settings] keystroke_max_chars` (default 8, at most 5000;
also under Settings > Engine in the GUI):

```toml
[settings]
keystroke_max_chars = 1000   # explicit opt-in to longer paced output
```

The insertion planner previews a route-native, manual-paste, or short paced-key
strategy from active daemon capabilities and compatibility consent. The GUI
estimates the whole transaction, including trigger erasure, replacement text,
delimiter handling, and a conservative cursor-travel bound. Runtime preflight
refuses a replacement longer than `keystroke_max_chars` before erasing the
trigger. These estimates use advertised throughput; they are not certified
latency guarantees. For
long configuration files and runbooks, use the preview's Copy action or
`wayexpand new` to write a file instead of typing the content through the
virtual keyboard. This does not make an unverified or non-atomic route safe for
automatic replacement.

Raising it trades speed for reach: anything you type while a long
replacement is being typed stops it partway (the evdev route cancels the rest
when physical input arrives), and a focus change or the insertion timeout
stops it too. A changed limit applies to the running daemon on the next
configuration reload.

This is intentional: the delay prioritizes correctness and reliability over speed
when the fast path isn't available.

**When does this happen?** It has been observed with xdg-desktop-portal-kde on
KWin 6.6. Treat that as a recorded compatibility observation, not a guarantee
for every later KWin or portal build; use `wayexpand doctor` and compositor
certification evidence for the session being deployed. As KDE Portal support
matures, `ei_text` support may become more common and this fallback may become
rarer; WayExpand makes no universal compositor claim until the relevant
certification evidence exists.

**Workaround:** If latency is intolerable:
- Use input-method-v2 instead (if your compositor supports it)
- Use shorter snippets (split long ones into smaller, composable pieces)
- Accept the tradeoff as a known limitation of your deployment

The implementation is isolated behind an optional backend crate and is not a
dependency of the platform-independent core. Both direct-socket and portal
paths use the pure-Rust `reis` protocol implementation; portal revocation and
compositor restart are surfaced as injector errors. The daemon retries
transport failures during startup and reconnects established output sessions
with bounded backoff. Because a transport failure may be ambiguous after
queued events were sent, the current replacement is not replayed. Portal
authorization failures are not retried automatically, avoiding repeated
consent prompts after a user denial.

## Input method

Input-method protocols may provide a clean UTF-8 insertion path for committed
text, but they are
not a universal global keyboard capture mechanism. Input-method-v2 is an
experimental protocol and compositor support is uneven; a Wayland session or
desktop name is not evidence that the protocol is available. Always use the
runtime probe and test the target compositor. The protocol specifies that the
keyboard grab is exclusive while active, so unsupported-key handling must
remain fail-closed.

WayExpand now contains an isolated `wayexpand-backend-input-method` source. It
binds the input-method manager and seat, creates an input-method object, grabs
the keyboard on activation, decodes the compositor keymap with xkbcommon, and
normalizes pressed keys into `InputEvent`s. It forwards printable text, Return,
and Tab with `commit_string` and the required commit serial. Backspace uses
the compositor's surrounding-text byte offsets, including selections and
multibyte UTF-8 characters; if that state is unavailable or invalid, it fails
closed rather than deleting a corrupt byte range.

**Key pass-through for unsupported keys:** Escape, arrows, F-keys, and
shortcut-like modified keys require a secondary libei injector. The daemon now
reattaches that injector when creating or reconnecting the input-method source.
Pass-through failures are reported as source errors and the daemon reconnects;
keys are not silently discarded. Modifier state is synthesized around the
passed-through key, but left/right modifier identity and compositor-specific
shortcut behavior are not yet certified.

**Held-key semantics:** Unsupported keys are passed through as lifecycle-aware
press/release events. Repeated compositor press notifications do not create
synthetic taps while a key is held; the virtual key remains down so the target
application can perform its normal key repeat. Held keys are released during
focus loss, deactivation, transport failure, and source teardown. If the
secondary injector is unavailable or fails, the source reports an error rather
than silently discarding the key.

Failed transitions remain in the pending queue until acknowledged by a
successful injector call. Teardown retries pending transitions and sends
compensating releases for keys whose delivery outcome is uncertain; queue
overflow preserves this safety ledger for the same cleanup path. This is a
best-effort process guarantee, not proof of compositor behavior: if the libei
transport is already unusable, release depends on the remote virtual-device
lifecycle and still requires real compositor testing.

This preserves key lifetimes in the source and injector, but
compositor-specific repeat rates, modifier identity, and shortcut behavior
still require real session certification. It is not evidence that every
compositor/client pair handles input-method-v2 identically.

Preedit handling and full compositor coverage remain open integration work,
so this source is opt-in and intentionally hidden behind `wayexpand setup --mode experimental`.
The daemon flag remains available for development and explicit automation, but
normal setup does not recommend or enable it. Replacements larger than the
protocol commit limit are rejected before any deletion is sent. Initial registry
discovery is deadline-bounded so a connected but unresponsive compositor cannot
hang one connection attempt indefinitely; retryable startup failures are retried
with bounded backoff until shutdown.

Activation starts with capture disabled until the compositor reports the
current content type. Password, hidden-text, sensitive-data, and unknown
content values remain disabled.

## Focused-window tracking (`app_filter`)

Expansions can be scoped to specific applications with `app_filter`. Bare
entries and `app_id_exact:<desktop-app-id>` entries are case-insensitive exact
matches against the focused window's normalized desktop app ID. For weaker,
explicitly opted-in behavior, use `app_id_glob:<pattern>` (`*` and `?`) or
`title_contains:<text>`. A title match is convenience matching, not a security
boundary. This needs to know which window is focused, and unlike
text capture and injection there is no Wayland protocol for that which
works across compositors: `wlr-foreign-toplevel-management-unstable-v1`
is implemented by some wlroots compositors (Sway, Hyprland), but WayExpand
does not currently ship a tracker using it. KDE Plasma's KWin implements
neither it nor the newer `ext-foreign-toplevel-list-v1` staging protocol.
KWin's authenticated window tracker can use the focused process's exact
same-user executable path when no desktop application ID is available. In
terminal-only mode, KWin resolves this fallback only for a focused window whose
compositor identity is not a usable app ID. The path is returned only by the
explicit `wayexpand debug --focused-app-id` command. Opt in to matching with
`process_executable_exact:/absolute/path` in `app_filter`,
`settings.terminal_apps`, or `settings.raw_capture_apps`; for example,
`process_executable_exact:/usr/bin/konsole`. This is an exact path comparison,
not a basename or title heuristic. It is currently KWin-specific. A missing,
cross-user, deleted, or changing process identity remains unknown and matching
fails closed. An executable allowlist limits which app receives expansion; it
does not provide password-field detection or make raw evdev capture private.
Input-method and virtual-keyboard protocol availability must be checked in the
configured session: KWin lets users select a virtual keyboard, and a headless
run without that session configuration is not a universal KWin capability
result. Use `wayexpand doctor` (or `wayland-info`) after configuring the
intended input method before choosing a route. See
[KDE's Virtual Keyboard settings](https://docs.kde.org/stable_kf6/en/kwin/kcontrol/kwinvirtualkeyboard/).

On GNOME, the WayExpand Shell extension authorizes tracker subscriptions by
session UID and the resolved executable's exact installation path (system
package locations or the versioned user payload directory), not by basename.
The isolated headless acceptance test can add its exact temporary client paths
through `WAYEXPAND_GNOME_TEST_CLIENTS`; production sessions leave this variable
unset. The default D-Bus `Subscribe()` reply
contains the app ID and opaque window instance ID, but no title. WayExpand
explicitly requests titles only to support configured `title_contains` filters.
This is a privacy reduction for unrelated session-bus clients, not isolation
from malicious software running as the same user: same-UID processes remain
within the desktop session's trust boundary.

`wayexpand-backend-kwin-window` bridges this gap for KDE Plasma the only
way currently available: KWin's scripting engine, reached over the session
D-Bus (`org.kde.kwin.Scripting`). This is the same mechanism community
tools like `kdotool` rely on for the same reason. The daemon loads a small
bundled script (`src/window-tracker.js`) that watches
`workspace.windowActivated` and calls back into a private D-Bus service
this process hosts for exactly that purpose, named uniquely per tracker
instance so reconnect cleanup cannot collide with its replacement. `loadScript`
returns before the resulting
`/Scripting/ScriptN` object is reliably reachable -- observed directly
against a live KWin 6.6 session, where calling `run()` immediately after
`loadScript` failed with "No such object path" for roughly the first second.
The tracker therefore retries `run()` with a short bounded backoff instead of
treating any specific KWin version as a fixed-delay contract.
While connected, it checks KWin's `isScriptLoaded` state every 500 ms on the
tracker worker (not the daemon's input loop).
If the script unloads or the health check fails, the supervisor clears the
cached window context and reconnects; app-filtered expansions fail closed
until the new script reports its initial active window.
The tracker uses `resourceClass` when present and falls back to KWin's
`desktopFileName` for native Wayland windows that do not provide a resource
class. A full desktop-file path is reduced to its basename, with a trailing
`.desktop` suffix removed, so the result can match configured app IDs.

An `app_filter`-scoped expansion fails closed rather than matching
everywhere when window tracking is unavailable (no tracker for this
compositor, or the KWin bridge failed to start): `ExpansionEngine` only
allows the match once a `WindowChanged` event has reported a window matching
the selected operator. Unfiltered
expansions are entirely unaffected.

Wlroots compositor support via `wlr-foreign-toplevel-management-unstable-v1`
is not currently shipped. A previous prototype was removed from the production
workspace because its constructor was disabled and its event-loop/lifetime
behavior was not ready for compositor use. A future implementation must be
feature-gated, tested against real Sway/Hyprland/river sessions, and prove
safe handle removal before it is reconsidered.

GNOME (Mutter) exposes no Wayland protocol or public D-Bus API for the focused
window: `org.gnome.Shell.Introspect` answers only allowlisted callers and
`org.gnome.Shell.Eval` is disabled. WayExpand therefore ships a small GNOME
Shell extension (`gnome-extension/wayexpand-window-tracker@cyberducttape.github.io`,
currently runtime-gated on GNOME Shell 46, 50, and 51) that runs inside GNOME
Shell. Other Shell majors are not declared compatible until they have a
version-pinned runtime gate. A tracker calls its
`Subscribe()` method and gets the focused window (app ID, title, Mutter window
ID) in the reply; later focus and title changes are pushed only to
subscribers, by calling `WindowChanged` on their private bus connection.
Nothing is broadcast on the session bus or stored. WayExpand accepts those
calls only from the process that owns `org.gnome.Shell`. The app ID is the
desktop-file ID (without `.desktop`), falling back to the Wayland app ID or
WM class for windows GNOME cannot match to an application; the Mutter window
ID is the exact window identity. When the overview or a Shell menu has focus,
no window is reported, so app filters fail closed.

`wayexpand setup` installs the extension for the user (or uses the system
copy that distribution packages install under
`/usr/share/gnome-shell/extensions/`), adds it to GNOME's enabled extensions,
and enables it at once when GNOME Shell already knows it. GNOME Shell on
Wayland loads a newly installed extension at the next login, which setup and
`wayexpand doctor` say explicitly. `scripts/test-gnome-e2e.sh` (CI's
`gnome-e2e`) runs the extension and tracker in a private, headless GNOME
Shell.

## wlroots virtual keyboard and uinput

These are fallback or compositor-specific mechanisms. Virtual-keyboard support
does not automatically provide global input capture. uinput requires explicit
permissions and layout-aware key event handling, so it must not be described as
Unicode-safe text insertion without further translation logic.

WayExpand now contains a real `wayexpand-backend-wlroots` output crate. It
connects through `wayland-client`, discovers `wl_seat` and
`zwp_virtual_keyboard_manager_v1`, uploads a per-operation XKB keymap, and sends
Unicode key events plus Backspace events. It intentionally does not claim to
capture input or support GNOME/KDE just because a Wayland session exists.

It can be exercised manually in a wlroots session with:

```sh
cargo run -p wayexpand-backend-wlroots --bin wlroots-type -- 'Hello 🙂'
```

This is an output diagnostic, not the daemon's automatic expansion path.
The daemon treats this route as paced synthetic keyboard output. The backend
does not advertise a measured throughput, so output deadlines use only the
base completion allowance.

UTF-8/Unicode-aware matching describes committed text only. WayExpand does not
support IME preedit/composition state; CJK, dead-key, Compose, Fcitx, and IBus
composition workflows must complete before a trigger is expected to match.
The backend caps each generated replacement at 8192 characters because every
character becomes synthetic keyboard traffic; larger replacements are rejected
before the trigger is erased. There is also a separate XKB keycode limit: each
distinct output character needs its own generated keycode, and the available
range currently permits at most 245 unique characters in one operation. A
replacement with repeated characters can therefore be long while remaining
valid, but a replacement containing more than 245 distinct Unicode characters
is rejected before the trigger is erased. The current implementation does not
chunk such output across multiple keymaps; chunking is a future improvement.
Registry discovery is deadline-bounded at startup so an unresponsive
compositor cannot leave a daemon connection attempt hanging forever.

## evdev (direct kernel input capture)

The evdev backend reads keyboard events directly from `/dev/input/event*`
devices, bypassing Wayland protocols entirely. This is necessary for compositors
(including KWin/KDE Plasma configurations observed on 6.6) that do not
implement `zwp_input_method_manager_v2` or
`zwp_virtual_keyboard_manager_v1`. The tradeoff is significant: evdev has
**no way to detect password fields or sensitive inputs**, since field semantics
are not available at the kernel level.

**Requires:** Explicit raw input-event access and the daemon flag
`--allow-evdev-sensitive-fields`, which acknowledges that password fields
cannot be detected. The default installer mode is
`--access=active-seat`, using logind/uaccess ACLs for keyboard-class event
nodes (`ID_INPUT_KEYBOARD`) without permanent group membership. The explicit
`--access=input-group` mode is a broader legacy fallback and may expose
additional input event devices through system group policy. See SECURITY.md
for the full security model and [EVDEV_ACCESS_DESIGN.md](EVDEV_ACCESS_DESIGN.md)
for the access-mode tradeoffs.

### Known Limitations

**Auto-repeat (key hold) handling:**
Kernel repeat events (code value == 2) are forwarded to the matcher as repeated
text, backspace, or boundary actions without changing XKB's physical key state.
They do not emit repeated global-hotkey actions. This matches non-exclusive
capture: the focused application receives native repeats while WayExpand sees
the same repeated input for trigger matching. Real-device behavior remains part
of compositor certification.

**Password-field protection:**
Because the kernel provides no field-type information, the matcher **never**
suspends matching in password fields. Sensitive-field detection is unavailable.
For that reason the daemon refuses to start an evdev source unless the operator
also passes `--allow-evdev-sensitive-fields`. This flag is an explicit
acknowledgement that expansions may run in password fields; it is not a claim
that evdev has acquired field awareness. Use `--source=input-method` when
password-field protection is required.

**Keyboard-layout synchronization:**
The evdev source translates keys only when it has a current compositor layout
snapshot. KDE's provider reads the active group from KWin and cross-checks its
effective layout and variant list against `kxkbrc`, or systemd-localed keyboard
properties when no per-user KDE layout file exists. Hyprland uses its
compositor device socket and accepts a snapshot only when all keyboards report
the same complete XKB rules/model/layout/variant/options and active layout
name, and that name uniquely matches the compiled XKB keymap. Layout changes
rebuild XKB state, reset the matcher, and replay held modifiers. Both providers
report `capture_layout_state=tracked` only while this contract is available;
missing or inconsistent state disables layout-dependent matching and hotkeys.
Sway currently remains unavailable: its input query reports active layout
display names but not the effective XKB rules/model/variant/options, so WayExpand
does not infer them from local configuration or system defaults.

**Non-exclusive capture and rapid typing:**
The focused application receives evdev key events independently of WayExpand.
For a match completed by a key-down event, the application can therefore see
the terminating character before WayExpand erases the matched text. The daemon
waits for held keys to be released and then requires a short quiet period; if
more input is observed while it is preparing the replacement, it abandons that
expansion rather than modifying a moving cursor. This bounds the common race,
but cannot make non-exclusive kernel capture atomic with application delivery.
Very high-speed typing can still produce an unexpanded trigger or overlapping
text. Use input-method-v2 where available for exclusive capture when this
correctness requirement is unacceptable.

This is the primary security limitation of the evdev backend. Before enabling
evdev, ensure your deployment model accepts this tradeoff (see SECURITY.md).
Evdev is also a best-effort timing backend: because capture is non-exclusive,
the application may receive the terminating key before WayExpand can erase and
replace the trigger. Serialized output can also take seconds on a keysym
fallback. Once that output finishes, WayExpand now clears matcher context and
discards captured evdev events until 40 ms of input quiet, preventing queued
keystrokes from being replayed as fresh triggers. During serialized,
non-atomic replacement the daemon also monitors evdev readiness and cancels
further synthetic output at the backend's next safety checkpoint when new
physical input arrives. The transaction is treated as uncertain, never
replayed, and captured input is quarantined. This can still leave a partial
replacement because already delivered physical keys and already injected
characters cannot be recalled. It therefore does not make replacement atomic
or guarantee that overlapping text is intact. See
[ADR 0001](adr/0001-evdev-best-effort-semantics.md) for the architectural
boundary and the risks of a future exclusive proxy.

**Keyboard layout handling:**
On KDE Plasma, evdev obtains the active XKB group from KWin's keyboard-layout
D-Bus interface and cross-checks KWin's effective layouts and variants
against `kxkbrc`, or systemd-localed keyboard properties when `kxkbrc` is
absent. Hyprland reports per-device keymap settings and active keymap names;
WayExpand compiles the reported keymap and requires every keyboard to agree
before matching. Layout changes reset matcher state and replay held modifiers
under the new group. If compositor data is absent, ambiguous, or inconsistent,
matching and hotkeys stay disabled. Sway currently has no evdev layout provider
because its input query does not expose sufficient effective XKB settings.

**Hotplug and layout limitations:**
Device discovery refreshes every five seconds, so a newly attached keyboard
may take up to that long to enter the capture set. KWin and Hyprland layout
state is polled and the XKB keymap is rebuilt on an active-group or
configured-layout change. Compose
and dead-key sequences are not tracked by evdev. This is a fallback path, not an
authoritative desktop keyboard integration.

**Probe detection is deliberately conservative about device identity:**
The evdev backend's `doctor` probe counts readable `/dev/input/event*` devices
as a permission signal, while the evdev connection performs the authoritative
keyboard-capability check. A readable event node is therefore not itself a
claim that a usable keyboard was found:

- `doctor` reports an evdev permission/device candidate
- Daemon starts and tries to use the device
- Device is a mouse, touchpad, or other input device
- Daemon finds no keyboard and fails to capture text

**Workaround:** If `wayexpand doctor` shows an evdev candidate but typing doesn't
expand:
1. List input devices: `ls -la /dev/input/event*`
2. Check which ones are keyboards: `cat /proc/bus/input/devices`
3. Verify at least one keyboard event node is readable: `getfacl /dev/input/eventN` for active-seat mode, or `ls -l /dev/input/event* | grep $USER` for legacy input-group mode
4. Restart daemon: `systemctl --user restart wayexpand-evdev.service`

The authoritative `EvdevSource::connect` path does inspect keyboard key
capabilities before starting capture, so a failed connection remains fail-closed.

**Hotplug and device tracking:**
The evdev source refreshes its keyboard-device discovery on a five-second fallback
interval, independently of the latency-sensitive input poll. New readable
keyboards are opened automatically, and devices that disappear are removed from
the active set. This supports USB keyboard hotplug without a daemon restart,
with up to one refresh interval of detection latency. Device identity is still
limited to the discovered evdev nodes; there is no administrator-managed
per-device allowlist or seat isolation.

For more details on evdev implementation, architecture decisions, and integration
testing, see the evdev backend crate source code.
