# Security policy

## Reporting a vulnerability

Please report suspected security vulnerabilities privately, not through a
public GitHub issue.

- Preferred: open a
  [private security advisory](https://github.com/cyberducttape/wayexpand/security/advisories/new)
  on GitHub ("Security" tab → "Report a vulnerability"). This reaches
  maintainers directly without disclosing the issue publicly.
- Include the affected version/commit, the backend(s) involved, reproduction
  steps or a proof of concept, and the impact you believe it has.

We aim to acknowledge new reports within 5 business days and to provide a
status update (triage result, expected timeline) within 14 days. If a report
is confirmed, we will coordinate a disclosure timeline with the reporter and
credit them in the release notes unless they prefer to stay anonymous.
Confirmed security fixes ship as patch releases as soon as they are ready
rather than on a fixed embargo schedule.

Do not report non-security bugs through the advisory process — use a regular
GitHub issue for those.

## Security model

WayExpand is intended to run as the unprivileged desktop user. It must not be
run as root.

For a concise table of protected, partially protected, and out-of-scope
threats, see [THREAT_MODEL.md](THREAT_MODEL.md).

The engine supports `InputEvent::FocusChanged { sensitive: true }`. An input
source that can identify password or other sensitive fields must emit that
event. The engine then clears its rolling buffer and ignores text until the
focus becomes non-sensitive again.

The input-method source begins every activation in this disabled state and
only enables capture after receiving a non-sensitive content type. Password,
hidden-text, sensitive-data, and unknown values are treated as sensitive.

The daemon never logs raw input, trigger names, or replacement contents. Normal
expansion logs contain only trigger character counts, erase counts, and
replacement byte counts. Diagnostic reports should still avoid including
configuration contents or typed text.

Configuration reload/startup failures are also logged without parser detail;
use `wayexpand doctor` when detailed configuration diagnostics are needed.

Configuration files must be regular files and must not be writable by group or
other users. Every ancestor directory must be owned by the current user or root
and must be non-group/world-writable unless it has sticky protection. Root-owned
sticky directories such as `/tmp` are permitted because their deletion/rename
policy protects entries. Sticky mode permits the standard `/tmp` permission
pattern but does not waive the ancestor ownership requirement. Configuration
files must be owned by the current user or root.

Symlinked configuration paths are resolved before ancestor validation and file
opening. This prevents a link swap from redirecting a trusted configuration
path into an untrusted directory.

Command expansions are opt-in executable user content. They invoke a named
program directly, never through a shell, with no stdin, discarded stderr,
bounded arguments, a maximum five-second runtime, and a 1 MiB UTF-8 stdout
limit. Non-zero exits, timeouts, invalid output, and oversized output fail
closed without emitting a replacement. A command can still have side effects
as the desktop user, so command-enabled configuration must remain protected by
the ownership and permission checks above.

**Important:** Command-backed expansions run under the systemd user service's
security hardening constraints (see `systemd/wayexpand.service`). The daemon
runs with `ProtectHome=read-only`, `ProtectSystem=strict`, memory write-execute
protection, and other sandboxing. This means a command that works when typed
manually may fail when invoked as a WayExpand expansion if it tries to:
- Write to `$HOME` or system directories
- Connect to the network (IPv4, IPv6)
- Access D-Bus services that require home-directory state or system-wide resources
- Access files outside `/tmp` or `/run`
- Perform other operations restricted by the systemd unit

**Host limitation:** `ProtectHome=`, `ProtectSystem=`, `PrivateTmp=`, and
`ReadWritePaths=` need a mount namespace, which a systemd *user* manager can
only create when unprivileged user namespaces are allowed. Where they are
restricted (for example Ubuntu with
`kernel.apparmor_restrict_unprivileged_userns=1`), systemd skips these
directives without an error, and only the process restrictions (no new
privileges, seccomp filters such as `RestrictAddressFamilies=` and
`MemoryDenyWriteExecute=`, and resource limits) apply. To check a running
daemon, compare `readlink /proc/<daemon-pid>/ns/mnt` with
`readlink /proc/self/ns/mnt`: identical values mean the filesystem
restrictions are not in effect.

The systemd unit is deliberately restrictive to limit the blast radius of a
compromised or misconfigured command. Do not relax the restrictions in
`systemd/wayexpand.service` or use a wrapper script as a way to bypass them.

The unit permits Unix domain sockets (`RestrictAddressFamilies=AF_UNIX`), which
are required for portals, D-Bus, and systemd user sockets. However, most D-Bus
services require access to the home directory or system-wide state, so they are
still blocked by the filesystem isolation settings above. A command that works
with D-Bus when run manually may fail under the daemon's constraints.

Named actions provide a supported policy-controlled path for selected SRE
commands that require capabilities unavailable to the daemon sandbox. The
integrated Action Broker is a separate same-user process and policy boundary;
it receives an explicit action name and enforces its executable, arguments,
environment, cwd, timeout, output limit, and optional audit result. The packaged
service denies IP networking. An action may also opt into Landlock read/write
path grants, enforced in the child process and refused when Landlock is
unavailable. Device isolation is not currently provided. Landlock is not a
boundary against malicious software running as the same UID; network access
remains global service policy rather than a per-action grant.
See
[`docs/ACTION_BROKER_ARCHITECTURE.md`](docs/ACTION_BROKER_ARCHITECTURE.md).

**GUI command previews:** With a reachable systemd user manager, direct
previews run in a transient user unit with the capture service's sandbox
directives, a minimal environment, and the command timeout. Named actions run
through the Action Broker. Without a systemd user manager, the GUI offers
**Run locally (advanced)**, which executes directly with the desktop user's
privileges; organization safe mode blocks this direct preview when the service
sandbox is unavailable. Direct previews are not included in the broker audit.
These modes are shown in the editor. The sandbox is only as strong as the host
allows, and it does not protect against malicious software running as the same
user. For details, see [`docs/OPERATIONS.md`](docs/OPERATIONS.md#command-backed-snippets).

The input-method source fails closed when it cannot safely pass through a
non-text key, preserve shortcut modifiers, or determine a UTF-8-safe Backspace
range from surrounding text. Unsupported non-text keys require the daemon's
libei pass-through injector; if that injector is unavailable or fails, the
source reports an error and reconnects rather than silently discarding keys.
Malformed protocol state remains fatal rather than risking text corruption.

The evdev source (`--source=evdev`) trades away a real security property the
other sources have: it reads keyboard events directly from the kernel
(`/dev/input/event*`) rather than through a Wayland protocol, so it has no way
to learn which application field has focus. It therefore **never** suspends
matching in password or other sensitive fields the way the input-method source
does. Only enable it where that tradeoff is acceptable.

Granting evdev access is a separate, explicit, root-requiring step
(`scripts/install-evdev-permissions.sh`, `--dry-run` first), never run
automatically by the user installers or base distro packages. WayExpand's udev
templates target udev keyboard-class event nodes. The active-seat mode uses
logind/uaccess ACLs and does not add the user to the broad `input` group. The
legacy input-group mode also adds the invoking user to `input`; that group may
grant broader raw input-event access depending on the distribution's default
input-device policy. Run `sudo scripts/install-evdev-permissions.sh --uninstall`
to reverse WayExpand's installed evdev policy and remove the invoking user from
`input` when appropriate.

This input-group path is the current legacy/simple access model, not the
long-term preferred architecture. A small device broker remains an investigation
candidate; session, seat, hotplug, and distribution behavior must be tested
before it can replace the current path. See
[docs/EVDEV_ACCESS_DESIGN.md](docs/EVDEV_ACCESS_DESIGN.md) for the
investigation plan and acceptance criteria.

Evdev also has a correctness limitation independent of permissions: capture is
non-exclusive, so the application can receive a terminating key before
WayExpand can erase and replace the trigger. The current quiet-period and
held-key mitigations are best-effort only; they do not make replacement atomic.
Do not use evdev where a lossless replacement guarantee is required. The
architectural boundary and the risks of an `EVIOCGRAB` proxy are documented in
[docs/adr/0001-evdev-best-effort-semantics.md](docs/adr/0001-evdev-best-effort-semantics.md).

### Insertion guarantees

WayExpand reports replacement assurance in `replacement_assurance` in
`wayexpand status --json`, in `doctor`, and prominently in the GUI and TUI.
The older `insertion_assurance` field remains as a coarse compatibility value.

- **Target-text-verified routes** (input-method-v2 and IBus) check the field's
  own surrounding text before erasing, and their edit is bound to that field
  by the protocol. This assurance does not claim that erase and insert are a
  single atomic transaction; IBus is not atomic.
- **Keystroke routes** (evdev capture with libei or wlroots output) erase with
  Backspace without seeing the field's text. They never erase unless the user
  sets `compatibility_mode` (in the primary config, the user layer, or the
  organization layer; curated packs cannot set it):
  - `tracked-focus` erases only while the window tracker (KDE) reports the
    same exact window, checked immediately before the first keystroke;
  - `untracked-focus` also erases when no tracker can confirm focus, and is
    labeled as such everywhere the route is shown.

  Live status calls a tracker-confirmed operation
  `focus-tracked-best-effort`, because it cannot establish that the cursor or
  surrounding text still matches the trigger. An operation with no confirmed
  focus is `unverified`; with `compatibility_mode = "off"`, the route is
  `blocked` and refuses before sending any erase or insertion keystroke.

  In every mode, a focus change detected between matching the trigger and
  typing refuses the expansion, as does a tracker that disconnects or stops.
- **Refusals** happen before anything is typed: nothing is erased or
  inserted, the expansion is not retried, and it is counted in
  `output_refused_total` with its reason in `output_last_refusal`.
- **Uncertain writes** are never retried. When a keystroke route fails or
  stops acknowledging partway through an expansion, the daemon counts it in
  `output_uncertain_total`, reports it in `doctor` and the GUI, and reconnects
  without replaying it.

Limits: focus tracking is only as current as the compositor's reports, so a
focus change in the last moments before typing, or one the compositor has
not yet reported, cannot be detected; and a keystroke route cannot detect a
cursor moved by mouse within the same window. These are the reasons keystroke
routes require explicit consent. Organizations can forbid them entirely with
`minimum_replacement_guarantee = "verified"` in safe mode.

### Security regression coverage

Targeted Rust regression tests exercise important process and storage
boundaries:

| Boundary | Representative coverage |
| --- | --- |
| Executable trust | `pinned_shebang_launch_uses_the_validated_inode_after_path_replacement` |
| Broker IPC | bounded/invalid frame tests and `ipc_concurrent_start_allows_only_one_owner` |
| Sensitive fields | input-method and IBus password/unknown-purpose tests, including an in-flight transition to a password field |
| Clipboard | bounded command environment, single worker, deadline, and disconnected-worker tests |
| Environment forwarding | broker tests prove an empty allowlist does not inherit client variables and client values cannot override server-owned values |
| Configuration saves | parent ownership/mode and symlink tests; `config_lock_rejects_hard_linked_lock_files`; concurrent-writer tests |
| Process cancellation | descendant cleanup, inherited-stdout, and cancellation tests in the command runtime and CLI form helper |
| Audit logs | private/single-link file, symlink-safe rotation, bounded required-audit backpressure, and rollback-failure tests |

These tests establish code-level properties for their tested conditions. They
do not certify sensitive-field behavior in a real compositor/client, validate
every filesystem race on every supported filesystem, or replace desktop
session testing. No compositor route is currently certified.

Backends must document their permission requirements explicitly:

- direct libei/EIS requires an explicitly configured `LIBEI_SOCKET`; portal
  libei requires explicit backend selection and an approved desktop
  remote-desktop session;
- wlroots virtual-keyboard is compositor-specific and requires the protocol
  to be available;
- uinput backend is not currently implemented;
- plugin execution must be disabled by default and sandboxed if added.

The control socket lives at `$XDG_RUNTIME_DIR/wayexpand.sock` (or the explicit
`WAYEXPAND_SOCKET` path), is resolved against a validated parent, created under
a restrictive `umask`, and finalized at mode `0600`. Its immediate parent and
all ancestors must be owned by the current user or root and must not be
group/world-writable unless the directory has the sticky bit set; this includes
the immediate socket parent, so a sticky directory such as `/tmp` is allowed.
Stale-socket cleanup requires both
the current UID and the original device/inode identity. The daemon refuses to
remove non-socket or differently owned paths. Service units also restrict
memory, tasks, file descriptors, and restart frequency.
They additionally isolate temporary files, devices, mounts, kernel interfaces,
process visibility, realtime scheduling, and syscall architecture through the
shipped systemd user units.

The daemon also checks Linux `SO_PEERCRED` on each control connection and
requires the peer UID to match the daemon UID. This authenticates the Unix user,
not an individual application: all ordinary processes under one UID share that
identity. The focused-window token is a freshness check, not an authorization
secret. Organizations that must prevent local applications from invoking
WayExpand through the socket can set
`[organization] safe_mode = true` and
`disable_control_socket_mutations = true`; this denies socket-based insert,
pause/resume, reload, stop, and usage-stat clearing while leaving read-only
status, focus, and explain requests available. It does not defend against
malicious code with the same UID using other user-session privileges such as
`systemctl --user`, editing user-owned configuration, or controlling the
desktop session.

## Sensitive data in process memory

The engine now uses `zeroize` for its live input history when characters are
backspaced, evicted, reset, or cleared after a focus/context transition. It
wipes its reusable Unicode matching scratch before reuse and shutdown, wipes
undo text when the undo record is invalidated, and does not retain undo copies
unless an undo chord is configured. Command-cache values are wiped when their
cache entries are dropped. Owned `InputEvent::Text` input and prefetched
clipboard values are wiped after processing or when consumed, replaced,
expired, or shut down. Shipped systemd services set `LimitCORE=0`.

These are best-effort controls over specific engine-owned allocations. They do
not erase immutable configuration snapshots, returned expansion results,
in-flight command/template strings, all clipboard-reader copies, stack spills,
registers, prior allocations left by reallocation, or data copied by the
allocator or operating system. GUI processes launched outside the shipped
systemd services do not inherit those units' core-dump limit. Do not treat
zeroization as a guarantee that secrets never appear in a process dump or can
never be recovered from memory.

Routine insert-refusal logs record only the fixed reason `focus_changed`; they
do not include compositor window identifiers or focus tokens. Direct command
stderr is drained to avoid blocking the child, then discarded. It is not
retained in `CommandError`, its `Display`/`Debug` representation, or the
daemon's failure trace. This favors confidentiality over exposing arbitrary
helper diagnostics; use the helper's own authorized administrative channel
when detailed diagnostics are required.

## Command execution architecture limitations

The current command-backed expansion architecture prioritizes safety at the cost
of restrictive resource constraints. It is not an Action Broker and is not an
SRE command-execution boundary:

**Current design:**
- Commands run directly in the daemon process, subject to systemd hardening
- `ProtectHome=read-only`, `ProtectSystem=strict`, no network access
- Prevents legitimate SRE tools from working: `kubectl`, `aws`, `vault`, `ssh`, etc.

The IBus integration keeps direct `program` commands disabled because IBus runs
outside the hardened daemon service. Named `action` commands are allowed only
through the integrated Action Broker, where broker availability and policy are
checked before execution. IBus therefore supports static expansions, direct
program commands remain disabled, and managed actions use the broker boundary.

**Enterprise/SRE use case conflict:**
A typical SRE might want:
```toml
[[expansion]]
trigger = ":shortlist"
replacement = ""  # typed text comes from the command
[expansion.command]
program = "kubectl"
args = ["get", "svc", "-o", "wide"]
```

This command fails in the current sandbox and cannot be fixed without materially
weakening the daemon's security posture for all users.

**Action Broker architecture (implemented for named actions)**

The current implementation separates concerns:

```
Capture/Match/Injection Process (extremely locked down)
         │
         │ constrained IPC (command name + args only)
         ▼
Action Broker (per-user policy engine)
         └─ allowed executables whitelist
         └─ network access policy
         └─ filesystem access policy
         └─ environment variable allowlist
         └─ command timeouts and resource limits
```

Benefits:
- Capture daemon stays extremely hardened, never touches the network or home dir
- Action broker runs with exactly the permissions needed for each environment
- Organization deploys `safe_mode = true` in config to completely disable command execution
- Command policies are inspectable and auditable
- Legitimate commands can be whitelisted by organization policy

This approach is particularly valuable for the target audience: SREs and system
administrators who need to trust WayExpand in hardened environments. Configure
the broker service and action catalog before using `action = "..."`; an
unavailable broker or missing action fails closed rather than falling back to a
direct command.
