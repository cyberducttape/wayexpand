# Operations Guide

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Operations**

---

## Configuration

## Installation

From a checkout, run:

```sh
./scripts/install-user.sh
```

This installs release binaries under `~/.local/bin`, copies both user units to
`~/.config/systemd/user`, installs an XDG desktop entry under
`~/.local/share/applications`, and creates the example configuration only when the
configuration path does not already exist. It does not overwrite an existing
configuration or start a service. After installation, run `systemctl --user
daemon-reload` and use `wayexpand doctor` before enabling one unit.
For a reviewed, supported session, `./scripts/install-user.sh --enable
--service=wayexpand-input-method.service` performs the user-manager reload and
activation explicitly. The installer refuses `sudo` execution because it must
install into the invoking user's home and user systemd manager.
Each unit validates the active configuration in `ExecStartPre` before starting
the daemon. Logs are sent to the journal with a stable identifier, so startup
and reload failures can be queried with `journalctl --user -u
wayexpand-input-method.service`.

The normal configuration path is:

```text
$XDG_CONFIG_HOME/wayexpand/expansions.toml
```

or, when `XDG_CONFIG_HOME` is unset:

```text
$HOME/.config/wayexpand/expansions.toml
```

Set `WAYEXPAND_CONFIG` when running under a service manager or test harness.
The daemon parses a changed file before replacing the active configuration. An
invalid file leaves the previous configuration active.

The optional settings section currently supports a bounded rolling buffer:

```toml
[settings]
max_buffer_chars = 128
```

The accepted range is 1–4096 characters.

Replacements may use safe built-in templates. They do not execute shell
commands or external programs:

```toml
[[expansion]]
trigger = ":today"
replacement = "Today is {{date}} ({{time}} UTC)."
description = "Current UTC date and time"
tags = ["date", "time"]
match_mode = "immediate"
```

`match_mode` defaults to `immediate`, preserving older configurations. Set it
to `word-boundary` when a trigger must not match inside a larger word. This
mode waits for the next character (or an explicit boundary) before inserting,
so a trigger followed by a letter cannot flash-expand and become part of a
larger word. Boundary checks are Unicode-aware and treat letters, numbers, and
`_` as word characters.

Available values are `{{date}}`, `{{time}}`, `{{datetime}}`, `{{username}}`,
`{{hostname}}`, `{{unix_timestamp}}`, `{{newline}}`, and `{{tab}}`. Unknown
variables make validation fail before a configuration reload is activated.
Template values are rendered only when an expansion matches.

For trusted, local system information, an expansion may run one program
without a shell:

```toml
[[expansion]]
trigger = ":kernel"
replacement = ""
description = "Running kernel release"

[expansion.command]
program = "uname"
args = ["-r"]
timeout_ms = 500
cache_ms = 60000
environment = "minimal"
pass_env = ["KUBECONFIG"]
```

Command stdout becomes the replacement after trailing newlines are removed.
`cache_ms` is optional and defaults to `0` (no caching); when set, successful
output is reused for that many milliseconds, which is useful for stable
system facts such as the kernel release.
Command processes receive a minimal environment by default (`HOME`, `USER`,
`PATH`, and `LANG`). Use `pass_env` for specific additional variables; use
`environment = "inherit"` only as an explicit trust decision because it passes
desktop-session variables and other daemon environment values to the child.
The daemon runs expansion commands and hotkey actions on a bounded background
queue, so a slow command does not block keyboard capture or control-socket
handling. Hotkey completion is logged asynchronously; a full action queue
drops the action with a warning rather than making the input loop wait. Output
is applied only while the trigger remains the newest processed input. If the
user types, changes focus, pauses, or reloads configuration first, the output
is discarded rather than risking deletion at a moved cursor. Successful stale
invocations still populate a configured cache for the next match.
The program inherits the daemon's user environment, receives no stdin, and
has stderr discarded. Output is capped at 1 MiB, arguments are bounded, and
the timeout is limited to 1–5000 milliseconds. A missing program, non-zero
exit, timeout, invalid UTF-8 result, or oversized result produces no
expansion and is never retried as shell text. Commands are deliberately not
shell snippets: pipes, redirects, and operators are passed as ordinary
arguments. Treat command-enabled configuration as executable user content and
keep the configuration ownership and mode protections enabled.

**Command-backed expansions and systemd sandbox:** Commands run under the
daemon's systemd-enforced sandbox (see `systemd/wayexpand.service` for details).
The daemon uses `ProtectHome=read-only`, `ProtectSystem=strict`, and other
restrictions that prevent commands from writing to `$HOME`, accessing the
network, or making unauthorized filesystem changes. A command that works when
typed manually may fail when triggered by WayExpand if it requires capabilities
the sandbox forbids.

Do not relax the daemon unit or use a wrapper script to bypass this boundary.
There is no supported SRE command path in the current direct-command model;
networked or credentialed workflows require the planned Action Broker. See
[`docs/ACTION_BROKER_DESIGN.md`](ACTION_BROKER_DESIGN.md) for its explicit
action allowlist, environment, cwd, network, timeout, output, and audit
requirements.

⚠️ **Important:** The GUI's Preview button does NOT run commands under the daemon's
systemd sandbox — it invokes them in the GUI process without those restrictions.
This means:

- `wayexpand-gui` preview can succeed while the live daemon expansion fails
- The preview validates expansion *output* but not the systemd sandbox constraints

To test a command under actual daemon sandbox restrictions, either:
1. Manually type the trigger and observe the result, or
2. Check `journalctl --user -u wayexpand-input-method.service` for daemon errors

For full details on sandbox behavior and command execution constraints, see
`SECURITY.md`'s section on command expansions.

Configuration files are capped at 16 MiB and 10,000 expansion entries to keep
reload memory and parsing cost bounded. Triggers and replacements containing
NUL characters are rejected because text-injection protocols cannot represent
them safely. The configured path must resolve to a regular file; directories,
FIFOs, and device nodes are rejected so a service reload cannot block on or
consume an unintended filesystem stream. Configuration files writable by group
or other users are rejected because they could alter text injected into another
application. Every ancestor directory must also be owned by the current user or
root and non-group/world-writable, unless it has sticky protection (as with a
root-owned `/tmp`), to prevent path replacement during reload. The file must be
owned by the current user or root. Symlinked configuration paths are
supported, but validation and loading use the resolved target path so the
target's ancestor directories are held to the same rules.

If an existing home configuration directory is group/world-writable, tighten
it before validation:

```sh
chmod go-w ~/.config
chmod go-w ~/.config/wayexpand
wayexpand doctor
```

This check is intentional: a writable ancestor could replace a trusted config
file between validation and reload.

## Diagnostics

Print the installed component versions when collecting a bug report:

```sh
wayexpand --version
wayexpand-daemon --version
```

```sh
wayexpand doctor [config]
wayexpand certify --json
wayexpand backend
wayexpand test ';;hello'
wayexpand preview ':today'
wayexpand list
wayexpand validate
wayexpand search email
wayexpand list --json
wayexpand preview ':today' --json
wayexpand set-enabled ':sig' off
wayexpand set-mode ':sig' word-boundary
wayexpand-ui
wayexpand-gui
wayexpand status
wayexpand status --json
wayexpand reload
wayexpand pause
wayexpand resume
wayexpand stop
RUST_LOG=wayexpand_daemon=info wayexpand-daemon
bash scripts/smoke-daemon.sh
bash scripts/test-doctor.sh
bash scripts/test-install-user.sh
```

`doctor` reports implementation state, configuration ownership/mode and parse
status, validates the control-socket parent and existing path permissions, and
performs non-invasive availability checks where possible, including a
writable-open check for `/dev/uinput`. A backend marked `RequiresPermission`
needs permissions, while `NotImplemented` means that environment access alone
does not make the backend selectable or usable. `doctor` exits nonzero when the
configuration is missing, unreadable, or invalid; lack of an active Wayland
session remains informational.

The stdin harness bounds each input line at 1 MiB. Oversized or invalid UTF-8
lines are discarded and logged without including their contents or stopping the
daemon.
The engine also bounds the number and total size of expansion results produced
from one text event, clearing the pending matcher state when that budget is
reached.

The daemon's `status` response includes `config_state=ok` when the active
configuration is accepted. `config_state=reload-rejected` means the daemon is
still using the last known-good configuration after an invalid, inaccessible,
or unstable edit. A `state=reconnecting` input-method status means matching is
disabled until the compositor connection is restored.

The smoke test exercises this status contract by rejecting an invalid edit,
restoring the valid file, and verifying recovery to `config_state=ok`.
Reload failures also emit a sanitized reason in the daemon log (for example,
`invalid TOML` or insecure permissions) without logging configuration paths,
triggers, or parser payloads.

The daemon exposes a user-owned `0600` Unix socket at
`$XDG_RUNTIME_DIR/wayexpand.sock` (or `WAYEXPAND_SOCKET`) for lifecycle
commands. Its immediate parent and all ancestors are validated for trusted
ownership and permissions before the path is resolved and bound. If no runtime
directory exists, the daemon continues without the socket and reports that
condition.

`wayexpand pause` disables matching and clears buffered trigger state without
stopping the daemon. `wayexpand resume` re-enables matching. The current pause
state is included in `wayexpand status`, which makes this useful as an
emergency privacy control before handling sensitive text.

Machine-readable list, preview, and status forms are available with `--json`.
They are intended for desktop UI integrations and scripts; status values such
as `paused` and booleans are emitted as typed JSON values rather than parsed
human-readable strings.

Espanso libraries can be converted without modifying the source file:

```sh
wayexpand import espanso ~/.config/espanso/match/base.yml > imported.toml
wayexpand validate imported.toml
```

SIGINT and SIGTERM request the same graceful shutdown path as `wayexpand stop`.
The shipped units automatically restart failures but stop trying after five
starts within 60 seconds, preventing a compositor incompatibility from causing
an unbounded restart loop.

## Services

Ordinary installers do not install the stdin harness as `wayexpand.service`.
That unit remains in the source tree only for deterministic daemon lifecycle
tests. The packaged services are expert-selected backend units; setup should
be the first step for a new user:

```sh
wayexpand setup
wayexpand status
wayexpand edit
```

`wayexpand setup` is an interactive onboarding command. It presents the
Recommended, Maximum compatibility, and Experimental modes, then selects the
safest detected path. `--yes` supports reviewed automation. It never changes
raw-input permissions or grants portal consent; those remain explicit security
decisions. Use `wayexpand explain-backend` for protocol-level diagnostics.

For expert manual backend selection, the input-method and evdev services remain
available:

```sh
systemctl --user enable --now wayexpand-input-method.service
systemctl --user status wayexpand-input-method.service
```

The input-method source retries both startup and later retryable Wayland
transport loss with bounded backoff, and marks the engine sensitive while
disconnected. It still fails permanently on unsupported protocol behavior. A
running harness unit is never proof that desktop capture is active. Enable only
one of these units at a time because both use the same control socket and
configuration path.

The stdin harness retries transport failures while starting a selected
`wlroots` or `libei` output session, and recovers the session after it has been
established. The failed replacement is deliberately not replayed: a transport
error can occur after the compositor has already accepted some or all of the
queued text, so replaying could duplicate user data. The matcher is reset, the
status reports `state=reconnecting`, and later input is held until a new backend
session is established. Unsupported compositor protocols fail immediately;
portal authorization failures are not retried automatically, so a user denial
does not cause repeated consent prompts. Validation and other non-retryable
injection errors remain fatal.

For a manual output-path test in a compositor that exposes the wlroots virtual
keyboard protocol:

```sh
wayexpand-daemon --backend=wlroots /path/to/expansions.toml
```

This still uses stdin as its input source; it is an integration harness, not a
global keyboard service.

The input-method-v2 path is explicit and currently the only integrated global
source:

```sh
wayexpand-daemon --source=input-method /path/to/expansions.toml
```

It must be tested on the target compositor. Unsupported non-text keys are
discarded individually and clear the pending trigger; missing or invalid
surrounding text remains a visible fail-closed error rather than an unsafe
deletion. Each Wayland connection attempt has a five-second
discovery deadline; retryable startup failures are retried with bounded
backoff, while protocol failures remain fatal. It is not enabled by the
systemd unit by default.

For a direct libei/EIS session, set `LIBEI_SOCKET` and opt in explicitly:

```sh
LIBEI_SOCKET=wayland-0-eis wayexpand-daemon --backend=libei /path/to/expansions.toml
```

Relative socket paths are resolved below `XDG_RUNTIME_DIR`. If
`LIBEI_SOCKET` is unset, explicitly selecting `--backend=libei` starts an XDG
RemoteDesktop portal session and may display an authorization dialog.

### Portal Permission Persistence

When using the RemoteDesktop portal (libei without direct `LIBEI_SOCKET`), a
restoration token is stored after the first successful connection at:

```text
$XDG_CONFIG_HOME/wayexpand/libei-portal-token
```

or:

```text
$HOME/.config/wayexpand/libei-portal-token
```

The token is stored with restricted permissions (mode 0600, user-only readable).
WayExpand validates the parent chain and file ownership, refuses token
symlinks, and writes through a durable atomic replacement (`fsync` before the
rename and on the containing directory). Save or validation failures are
reported as portal connection errors rather than silently discarding the token.
On subsequent daemon starts, the stored token is passed to the portal to restore
the previous session, skipping the user consent dialog if the token is still valid.

The service units explicitly set `WAYEXPAND_PORTAL_TOKEN_PATH` to
`%h/.config/wayexpand/libei-portal-token`, so their systemd writable-path grant
remains correct even when `XDG_CONFIG_HOME` is customized. Standalone daemon
invocations may set this variable to an explicitly trusted location; the CLI
uses the same variable for portal status and reset operations.

If the token expires or the user revokes permissions, the portal will display the
authorization dialog on the next connection attempt. To manually reset permissions,
use `wayexpand portal reset`, which applies the same path and ownership checks.

## Native backend prerequisites

The direct libei backend is built from the pure-Rust protocol implementation
and requires a compositor/EIS server plus an explicitly configured
`LIBEI_SOCKET`. The portal-backed path uses the desktop's XDG RemoteDesktop
portal and requires user consent (one-time, with token persistence). Portal 
revocation or compositor restart is reported as an injector failure; the stdin 
daemon reconnects the output session with bounded backoff. Because a transport 
failure may be ambiguous after queued events were sent, the current replacement 
is not replayed. Direct EIS handshakes and device discovery have a five-second 
I/O deadline so a stale endpoint cannot hang daemon startup indefinitely.
## Monitoring and health checks

Use the machine-readable doctor command from a service check or fleet probe:

```sh
wayexpand doctor --json ~/.config/wayexpand/expansions.toml
```

The JSON document contains `healthy`, `config`, `control_socket`, `wayland`,
and `backends` fields. It exits non-zero when the configuration is invalid or
when a configured runtime socket is missing. It intentionally does not run
compositor connection probes, so checks remain bounded and side-effect free.

For a running daemon, prefer the control API for liveness and operational
state:

```sh
wayexpand status --json
```

The control socket is user-owned and mode `0600`; do not expose it through a
shared filesystem or proxy it over a network.
