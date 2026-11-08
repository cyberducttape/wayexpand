# Getting Started with WayExpand

**Navigation:** [Home](../README.md) > **Getting Started**

---

Welcome! Start with the graphical setup guide. It checks the installed
integration, explains any choices that need your consent, and distinguishes
configuration from a verified real-application test.

## Graphical first run

### 1. Install and open WayExpand

Use a distribution package where one is available, then open **WayExpand**
from your applications menu. See the [packaging guide](PACKAGING.md) for
current availability: the Ubuntu daily package is experimental, and a Fedora
package channel is not currently published. Source installation is intended
for developers and packagers.

If the application is already installed but has no launcher, run
`wayexpand-gui`.

### 2. Follow setup and add a snippet

Follow the setup screen's route recommendation. It checks capabilities and
explains permissions before applying changes; it does not enable raw keyboard
capture automatically. The GUI status distinguishes **Installed**,
**Configured**, and **Verified working**.

In the editor, select **Add snippet**, enter a trigger such as `;;hello` and a
replacement such as `Hello, world!`, then save. You can use any enabled,
harmless static snippet for verification; setup does not require the example
trigger to exist.

### 3. Verify in a real application

Choose **Desktop Integration Test** and follow the prompt in a disposable,
non-sensitive text field. Inspect the result and confirm only if it exactly
matches the expected replacement. WayExpand never presses Enter or retries an
insertion whose outcome is uncertain. Until you confirm an exact result, the
GUI reports **Configured, untested**; afterward it reports **Working in the
tested application**. That result applies only to the tested app and session.

If no automatic route is available, Quick Picker remains available in
copy-only mode: choose a snippet, switch to the target application, and paste
it yourself. It does not enable raw capture or attempt insertion.

## Advanced CLI setup and route diagnostics

Backend readiness and a matcher preview do not prove that desktop typing or
replacement works in a particular application. Check the
[support matrix](SUPPORT_MATRIX.md) for evidence about your compositor and
route.

### 1. Check the session

```sh
wayexpand doctor
wayexpand explain-backend
```

Use the backend combination that `doctor` prints as usable. Backend support is
compositor-dependent; an `Implemented` line means the code exists, not that
your current session has advertised the protocol.

When the daemon is running, `doctor` reports the active session's capture,
injection, focus tracking, password-field signal, and replacement assurance
separately. Each entry includes what the daemon observed and a suggested next
step. These are live capability reports, not proof that every application
handles real typing correctly; `doctor --certify` is an isolated smoke test
and does not certify your active desktop session.

Active IME/preedit, dead-key, and Compose composition is not handled: triggers
are matched against committed text. App-filtered snippets depend on the
specific route's focus tracker and fail closed when app identity is missing,
stale, or unbound to the active input context. The GNOME Shell extension
provides authenticated focus updates to subscribing WayExpand services; the
IBus service binds those updates to its active input context. That integration
is implemented but has not passed real-session certification. KWin provides a
separate tracker for supported daemon routes. Sway, Hyprland, and river do not
currently provide window tracking. See the
[support matrix](SUPPORT_MATRIX.md) before relying on app-scoped workflows.
The GNOME focus binding is enabled only for the GNOME IBus route; IBus on other
desktops does not currently receive application identity and therefore
suppresses app-filtered snippets.

### Check insertion compatibility in isolation

Before relying on a route, run the reproducible desktop smoke check:

```sh
wayexpand doctor --certify
```

It starts a private, headless KWin session and verifies what isolated Wayland
test clients actually receive. The report shows the negotiated injection
route and marks each scenario **Pass**, **Fail**, or **Not tested**. Save the
machine-readable report when sharing results:

```sh
wayexpand doctor --certify --json > wayexpand-compatibility.json
```

This test does not change the active desktop or configuration. It simulates
keyboard capture, so it does not certify physical key capture, password-field
behavior, suspend/resume, or your configured desktop session. An overall
“Partial certification” is expected for this smoke test; use the
[certification guide](CERTIFICATION.md) for real-session release evidence.

### Optional: configure raw evdev

The guided `wayexpand setup` command recommends a detected route and explains
its limits. Automatic mode intentionally leaves raw evdev disabled, even when
`/dev/input` is readable. If you explicitly choose evdev and accept its global
keyboard visibility/no-password-field-awareness tradeoff, install the
permission rule and start that service:

```sh
sudo ./scripts/install-evdev-permissions.sh --access=active-seat
systemctl --user enable --now wayexpand-evdev.service
wayexpand status --json
```

The status should report `state=connected`. Stdin is only a test harness, not
normal desktop capture. Input-method-v2 is an explicit experimental opt-in;
read its key pass-through and held-key/repeat limitations before enabling its
service. For any route, read the security tradeoffs in
[SECURITY.md](../SECURITY.md).

### CLI-based verification (optional)

```sh
wayexpand test ';;hello'
```

This is a matcher-only check: it confirms the saved configuration can match
the trigger and render the replacement. It does not send input to your desktop
or prove that an application will receive a replacement.

For the desktop check, focus a disposable, non-sensitive text field and run:

```sh
wayexpand debug --test ';;hello'
```

If `;;hello` is not in your library, substitute any enabled static trigger
listed by `wayexpand list`; command-backed and form-driven snippets are not
used for this harmless insertion check.

Follow its prompt, type the trigger once, inspect the resulting text, and
confirm only if it exactly matches the expected replacement. WayExpand never
presses Enter or retries uncertain output. The GUI offers this as **Desktop
Integration Test** during setup. Until you confirm an exact result, report the
state as **Configured, untested**; after confirmation, it reports **Working in
the tested application**. That result applies only to the tested app and
session. It does not certify other applications or the compositor. If the
matcher check instead prints `no expansion matched`, inspect the trigger with
`wayexpand list` and `wayexpand validate` before debugging the backend.

If no automatic route is available, the first-run guide offers Quick Picker in
copy-only mode. It does not enable raw keyboard capture or attempt insertion;
choose a snippet, switch back to the target application, and paste it yourself.

### 5. Add an app-filtered snippet (optional)

Add this in the TUI/GUI or to the TOML file:

```toml
[[expansion]]
trigger = ";;ticket"
replacement = "Investigating this now."
description = "Support response for the ticketing app"
app_filter = ["com.example.TicketApp"]
```

App filters only work when the current compositor has a working window tracker.
They fail closed when tracking is unavailable. Verify the exact application ID
with the desktop's window-inspection tools and confirm support in
[SUPPORT_MATRIX.md](SUPPORT_MATRIX.md); use a global snippet if the tracker is
experimental or unavailable.

For the output-driven recovery flow, go directly to the
[Troubleshooting guide](TROUBLESHOOTING.md).

### Architecture note: aarch64

The release workflow supports native `x86_64` and `aarch64` builds. Published
package and archive availability varies by version and architecture, and
current builds remain experimental until real-desktop certification. Check
the [packaging guide](PACKAGING.md) for the current installation matrix.

## Which Desktop Are You Using?

### KDE Plasma Wayland (KWin) — implementation candidate

Open WayExpand from the applications menu and follow its setup guide. This is
the most complete implementation path, but it still requires live session
testing and is not certified by the current support matrix. The KWin window
tracker is implemented for supported daemon routes; app-restricted snippets
must still be tested in the intended application. KWin 6.6 has live-session
observations in the backend notes, but newer releases are not assumed
compatible based on version alone.

Use `wayexpand doctor` only if the GUI identifies a setup problem. Raw evdev
is a separate opt-in compatibility route with global keyboard visibility and
no password-field signal; do not enable it merely because another route is
unavailable. See [SECURITY.md](../SECURITY.md) before selecting it.

---

### Sway / Hyprland / river — experimental

Wlroots-based compositor support has active development in progress. Window
tracking is not currently shipped.

**Current status:**
- Text capture and injection paths are implemented (via input-method-v2 or
  evdev), but have not been certified on these compositors
- ⏳ Window tracking (app_filter) is not shipped on wlroots; the integration remains future experimental work
- ⚠️ Password field detection requires input-method-v2

**Getting started:** Open the GUI and review the route it recommends. Do not
start the evdev daemon directly as a general fallback: it is opt-in, has no
password-field signal, and is not certified on these compositors. Developers
testing an experimental route should follow
[INTEGRATION_TESTING.md](INTEGRATION_TESTING.md).

**Known limitations:**
- Automatic mode leaves evdev disabled even when `/dev/input` is readable
- Input-method-v2 is explicit/experimental: non-text pass-through depends on
  the libei injector and remains uncertified for every compositor and shortcut
  combination
- Evdev uses active-seat/logind uaccess by default; the explicit
  `--access=input-group` fallback grants broader permanent input-group access.
  Both modes have no password-field signal and are best-effort under rapid typing

**Want to help test?**
- Run `wayexpand doctor` and share the output on our [GitHub issues](https://github.com/cyberducttape/wayexpand/issues)
- Test snippets and report what works/breaks
- See [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) for detailed testing

---

### GNOME Shell Wayland — limited/uncertified

Limited, uncertified support. GNOME does not expose the window-tracking
protocol WayExpand needs, so WayExpand uses its GNOME Shell extension to
provide focus information to subscribing services.

**Current input routes:**
- Mutter does not currently advertise `zwp_input_method_manager_v2`, so
  WayExpand cannot use its input-method-v2 source on GNOME.
- The separate WayExpand IBus engine can be used: set it up with
  `wayexpand setup --backend ibus` and check it with `wayexpand ibus status`
  (see [IBUS_ENGINE.md](IBUS_ENGINE.md)).
  IBus content-purpose signals can suppress matching in sensitive fields,
  while trigger deletion is verified against surrounding text but is not
  atomic. This route is experimental and not production-certified.

**Window-specific snippets (`app_filter`):**
- `wayexpand setup` installs and enables the WayExpand GNOME Shell extension
  (GNOME 45 or newer, including Ubuntu 24.04 and later). A new extension may
  require a GNOME logout/login before Shell loads it; `wayexpand doctor --gnome`
  distinguishes that case from an unavailable Shell query or extension error.
- The extension sends authenticated focus updates to subscribing WayExpand
  services. The IBus service binds the focused GNOME window to the active
  IBus input context and clears app identity on tracker loss or focus change.
  This integration is implemented but has no real-session certification.
- The extension exposes app IDs and window titles only to authorized
  subscribers; it does not broadcast or persist them.

**What doesn't work:**
- ❌ Atomic replacement and production-certified password-field protection

**Recommendation:**
- Prefer global snippets unless you have verified the IBus app-filtered path
  in the intended application; both paths remain uncertified
- See [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for compatibility status

---

## Snippet examples

The graphical editor is the recommended way to create and update snippets.
For configuration automation, the same library can be managed as TOML and
checked with the CLI. Common patterns:

**Email signature:**
```toml
[[expansion]]
trigger = ";;sig"
replacement = """Best,
Your Name"""
```

**One snippet, several triggers (aliases):**
```toml
[[expansion]]
trigger = ";;addr"
aliases = [";;address", ";;office"]
replacement = "1 Main Street"
```
Every alias uses the snippet's match mode and options, and an alias may not
collide with any other trigger.

**Local usage statistics:** `wayexpand stats` shows how many expansions you
used recently, keystrokes avoided, your most-used snippets, snippets unused for
90 days, and triggers likely to fire by accident (for example plain words in
immediate mode). The daemon records only snippet IDs, counts, and dates, never
text, in `usage-stats.json` next to your configuration; nothing leaves your
machine. `wayexpand stats --clear` deletes it, and
`[settings] usage_stats = false` stops recording. If a daemon clear request
times out, its result is unknown; check `wayexpand stats` before retrying
because the queued clear may still complete. Statistics are asynchronous and
best-effort, so the file can lag recent expansions or omit events after a
reported writer failure or shutdown deadline; it is not an exact audit ledger. Check
`wayexpand status --json` for recorded, persisted, and dropped event counters.

See [TEMPLATES.md](TEMPLATES.md) for every variable, including snippet
includes, allowlisted environment variables, and the opt-in clipboard.

**Date (auto-generated):**
```toml
[[expansion]]
trigger = ";;date"
replacement = "{{local_date}}"  # Today's date in your time zone; {{date}} is UTC
```

**Code snippet:**
```toml
[[expansion]]
trigger = ";;func"
replacement = """def my_function(x):
    return x * 2"""
```

See [COMPATIBILITY.md](COMPATIBILITY.md) for the configuration format, and [FOR_SYSADMINS.md](FOR_SYSADMINS.md) for 30+ real-world variable examples.

---

## Troubleshooting

**Snippets aren't expanding?**
1. Run `wayexpand doctor` to check backend status
2. Verify the daemon is running: `systemctl --user status wayexpand-input-method.service`
3. Check logs: `journalctl --user -u wayexpand-input-method.service -n 20`
4. Run the matcher check, then complete **Desktop Integration Test** in a
   disposable text field. Treat the route as working only in the application
   where you confirmed the exact result; see the support matrix for what has
   and has not been certified.

**Password field protection not working?**
- This requires input-method-v2 backend (shows in `wayexpand doctor`)
- evdev capture (`--source=evdev`) has no password detection
- input-method-v2 suppresses matching until it receives a content-purpose
  update and treats unknown purposes as sensitive; compositor support remains
  uncertified until validated in the certification matrix

**Keys are lost (Escape, arrows, F-keys)?**
- This is input-method-v2's limitation on some compositors
- Use evdev as fallback only after explicitly accepting its sensitive-field limitation: `wayexpand-daemon --source=evdev --allow-evdev-sensitive-fields ~/.config/wayexpand/expansions.toml`
- Remember: evdev has no password protection. Its installer uses active-seat
  ACLs by default; choose `--access=input-group` only when the broader legacy
  grant is acceptable.

**See Also:**
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) - What's verified vs. experimental
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) - doctor output → fix
- [PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md) - matcher and large-library tuning
- [OPERATIONS.md](OPERATIONS.md) - Systemd management and troubleshooting
- [BACKENDS.md](BACKENDS.md) - Backend protocols and compatibility

---

## Next Steps

- **Learn the config format:** [Configuration Limits](CONFIGURATION_LIMITS.md) and [Compatibility](COMPATIBILITY.md)
- **Understand the architecture:** [BACKENDS.md](BACKENDS.md)
- **Report bugs:** See [SECURITY.md](../SECURITY.md#reporting-a-vulnerability) for security issues, or open a GitHub issue for bugs

Welcome to WayExpand! 🎉
