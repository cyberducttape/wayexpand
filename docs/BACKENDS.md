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
layout-dependent -- only characters the current layout can actually produce
via an unshifted or Shift-level keysym are typeable -- and a replacement
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

**Character insertion latency (ei_keyboard fallback):**
When the EIS server provides only `ei_keyboard` (no `ei_text`), the backend
synthesizes individual key presses for each character. To ensure reliability
and prevent key merging at the compositor, a safe delay is added between each
synthesized character. This means:

- **Normal case** (ei_text available): Sub-millisecond insertion (fast)
- **Fallback case** (ei_keyboard only): ~10-50ms per character (slower)

For short snippets (signatures, small templates), this is imperceptible.
For long replacements (multi-paragraph code blocks, long administrative text),
the total insertion time becomes noticeable:

- 100-character replacement: ~1-5 seconds
- 1000-character replacement: ~10-50 seconds

This is intentional: the delay prioritizes correctness and reliability over speed
when the fast path isn't available.

**When does this happen?** Observed with xdg-desktop-portal-kde on KWin 6.6.
As KDE Portal support matures, `ei_text` support should become universal and
this fallback will become rare.

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

Input-method protocols may provide a clean UTF-8 insertion path, but they are
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
closed rather than deleting a corrupt byte range. Escape and other unsupported
non-text keys are discarded individually and clear the matcher because
silently interpreting them would be unsafe. The individual grabbed event may
be lost; the source does not claim general non-text pass-through and does not
restart the daemon for ordinary unsupported keys. Preedit handling, full
non-text pass-through, and compositor coverage
remain open integration work, so this source is opt-in with
`--source=input-method`. Replacements larger than the protocol commit limit
are rejected before any deletion is sent. Initial registry discovery is
deadline-bounded so a connected but unresponsive compositor cannot hang one
connection attempt indefinitely; retryable startup failures are retried with
bounded backoff until shutdown.

Activation starts with capture disabled until the compositor reports the
current content type. Password, hidden-text, sensitive-data, and unknown
content values remain disabled.

## Focused-window tracking (`app_filter`)

Expansions can be scoped to specific applications with `app_filter` --
a list of case-insensitive substrings matched against the focused window's
app id or title. This needs to know which window is focused, and unlike
text capture and injection there is no Wayland protocol for that which
works across compositors: `wlr-foreign-toplevel-management-unstable-v1`
covers wlroots compositors (Sway, Hyprland), but KDE Plasma's KWin
implements neither it nor the newer `ext-foreign-toplevel-list-v1`
staging protocol -- a deliberate privacy stance, the same one that keeps
KWin off `zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1`
(see the evdev fallback above).

`wayexpand-backend-kwin-window` bridges this gap for KDE Plasma the only
way currently available: KWin's scripting engine, reached over the session
D-Bus (`org.kde.kwin.Scripting`). This is the same mechanism community
tools like `kdotool` rely on for the same reason. The daemon loads a small
bundled script (`src/window-tracker.js`) that watches
`workspace.windowActivated` and calls back into a private D-Bus service
this process hosts for exactly that purpose, named uniquely per process
(`org.wayexpand.WindowTracker.pid<pid>`) so multiple daemon instances do
not collide. `loadScript` returns before the resulting
`/Scripting/ScriptN` object is reliably reachable -- observed directly
against a live KWin 6.6 session, where calling `run()` immediately after
`loadScript` fails with "No such object path" for roughly the first
second -- so starting the tracker retries `run()` with a short bounded
backoff rather than guessing a fixed delay.

An `app_filter`-scoped expansion fails closed rather than matching
everywhere when window tracking is unavailable (no tracker for this
compositor, or the KWin bridge failed to start): `ExpansionEngine` only
allows the match once a `WindowChanged` event has reported a window whose
app id or title actually contains one of the filter strings. Unfiltered
expansions are entirely unaffected.

Wlroots compositor support via `wlr-foreign-toplevel-management-unstable-v1`
is not currently shipped. A previous prototype was removed from the production
workspace because its constructor was disabled and its event-loop/lifetime
behavior was not ready for compositor use. A future implementation must be
feature-gated, tested against real Sway/Hyprland/river sessions, and prove
safe handle removal before it is reconsidered. GNOME (Mutter) exposes no
equivalent bridge without a shell extension, so no fully evidence-based path
exists there today.

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
The backend caps each generated replacement at 8192 characters because every
character becomes synthetic keyboard traffic; larger replacements are rejected
before the trigger is erased. Registry discovery is deadline-bounded at startup
so an unresponsive compositor cannot leave a daemon connection attempt hanging
forever.

## evdev (direct kernel input capture)

The evdev backend reads keyboard events directly from `/dev/input/event*`
devices, bypassing Wayland protocols entirely. This is necessary for compositors
(notably KWin/KDE Plasma as of 6.6) that do not implement `zwp_input_method_manager_v2`
or `zwp_virtual_keyboard_manager_v1`. The tradeoff is significant: evdev has
**no way to detect password fields or sensitive inputs**, since field semantics
are not available at the kernel level.

**Requires:** Membership in the `input` group, which grants raw keyboard access
to **all keystrokes** system-wide (not just WayExpand's). See SECURITY.md for
the full security model and explicit opt-in procedure. This is the current
legacy/simple permission model; active-seat ACLs and a device broker are future
investigation work documented in [EVDEV_ACCESS_DESIGN.md](EVDEV_ACCESS_DESIGN.md).

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
replace the trigger. The quiet-period mitigation cannot make this atomic. See
[P0_3_DECISION_REQUIRED.md](P0_3_DECISION_REQUIRED.md) for the architectural
boundary and the risks of a future exclusive proxy.

**Keyboard layout handling:**
The evdev source uses the system XKB keymap (typically loaded at session start).
If the active layout changes during a session, the mapper continues using the
original layout until the daemon is restarted. For fixed-layout deployments,
this is not an issue. For multi-layout switchers, consider:

- Restarting the daemon after switching layouts: `systemctl --user restart wayexpand-evdev.service`
- Using input-method-v2 instead, which learns layout changes from the compositor
- Using only ASCII triggers and replacements (no layout-dependent characters)

**Probe detection doesn't verify keyboards:**
The evdev backend's `doctor` probe counts any readable `/dev/input/event*` device
as potentially valid. It does NOT verify that a readable device is actually a
keyboard (has EV_KEY capability bits). This can produce a false-positive
diagnostic result:

- `doctor` reports "evdev capture ready" ✓
- Daemon starts and tries to use the device
- Device is a mouse, touchpad, or other input device
- Daemon finds no keyboard and fails to capture text

**Workaround:** If `wayexpand doctor` says evdev is ready but typing doesn't
expand:
1. List input devices: `ls -la /dev/input/event*`
2. Check which ones are keyboards: `cat /proc/bus/input/devices`
3. Verify at least one is readable: `ls -l /dev/input/event* | grep $USER` (for input group membership)
4. Restart daemon: `systemctl --user restart wayexpand-evdev.service`

**Future improvement:** The probe should inspect EV_KEY/key capability bits and
report actual candidate keyboards specifically.

**Hotplug and device tracking:**
The evdev source refreshes its keyboard-device discovery on every bounded poll.
New readable keyboards are opened automatically, and devices that disappear are
removed from the active set. This supports USB keyboard hotplug without a daemon
restart. Device identity is still limited to the discovered evdev nodes; there
is no administrator-managed per-device allowlist or seat isolation.

For more details on evdev implementation, architecture decisions, and integration
testing, see the evdev backend crate source code.
