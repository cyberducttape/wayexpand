# Security policy

## Reporting a vulnerability

Please report suspected security vulnerabilities privately, not through a
public GitHub issue.

- Preferred: open a
  [private security advisory](https://github.com/itchyitchy123/wayexpand/security/advisories/new)
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
- Connect to the network or D-Bus
- Access files outside `/tmp` or `/run`
- Perform other operations restricted by the systemd unit

The systemd unit is deliberately restrictive to limit the blast radius of a
compromised or misconfigured command. If a command needs capabilities the
daemon's sandbox forbids, either relax the restrictions in
`systemd/wayexpand.service` (requires manual editing) or run a less restricted
wrapper script. The `wayexpand-gui` preview feature can help identify these
issues: a command that fails in Preview due to sandbox restrictions will also
fail when triggered during typing.

The input-method source fails closed when it cannot safely pass through a
non-text key or determine a UTF-8-safe Backspace range from surrounding text.
Unsupported ordinary non-text keys are discarded individually and clear the
pending matcher state; malformed protocol state remains fatal rather than
risking text corruption.

The evdev source (`--source=evdev`) trades away a real security property the
other sources have: it reads keyboard events directly from the kernel
(`/dev/input/event*`) rather than through a Wayland protocol, so it has no
way to learn which application field has focus. It therefore **never**
suspends matching in password or other sensitive fields the way the
input-method source does. Only enable it where that tradeoff is acceptable.
It also requires the user to be in the `input` group, a broader grant than
the Wayland sources need, since that group can read every keystroke typed
anywhere in the session -- including other users' sessions and password
prompts -- not only ones passed to WayExpand's own matcher. Granting this is
a separate, explicit, root-requiring step
(`scripts/install-evdev-permissions.sh`, `--dry-run` first), never run
automatically by the user installers, which install
`udev/71-wayexpand-evdev.rules` (reasserting the standard
`SUBSYSTEM=="input", GROUP="input"` default most systemd distributions
already ship, rather than granting anything broader) and add the invoking
user to `input`. Run `sudo scripts/install-evdev-permissions.sh --uninstall`
to reverse it.
This input-group path is the current legacy/simple access model, not the
long-term preferred architecture. Active-seat ACLs through logind/udev and a
small device broker are investigation candidates; their session, seat,
hotplug, and distribution behavior must be tested before either can replace
the current path. See [docs/EVDEV_ACCESS_DESIGN.md](docs/EVDEV_ACCESS_DESIGN.md)
for the investigation plan and acceptance criteria.

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
group/world-writable; a root-owned sticky ancestor such as `/tmp` is allowed,
but never as the immediate socket parent. Stale-socket cleanup requires both
the current UID and the original device/inode identity. The daemon refuses to
remove non-socket or differently owned paths. Service units also restrict
memory, tasks, file descriptors, and restart frequency.
They additionally isolate temporary files, devices, mounts, kernel interfaces,
process visibility, realtime scheduling, and syscall architecture through the
shipped systemd user units.

## Command execution architecture limitations

The current command-backed expansion architecture prioritizes safety at the cost
of restrictive resource constraints:

**Current design:**
- Commands run directly in the daemon process, subject to systemd hardening
- `ProtectHome=read-only`, `ProtectSystem=strict`, no network access
- Prevents legitimate SRE tools from working: `kubectl`, `aws`, `vault`, `ssh`, etc.

**Enterprise/SRE use case conflict:**
A typical SRE might want:
```toml
[[expansion]]
trigger = ":shortlist"
command = "kubectl get svc -o wide"
```

This command fails in the current sandbox and cannot be fixed without materially
weakening the daemon's security posture for all users.

**Planned mitigation: Action Broker architecture (v1.3+)**

Future versions will separate concerns:

```
Capture/Match/Injection Process (extremely locked down)
         │
         │ constrained IPC (command name + args only)
         ▼
Optional Action Broker (per-user policy engine)
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
administrators who need to trust WayExpand in hardened environments.
