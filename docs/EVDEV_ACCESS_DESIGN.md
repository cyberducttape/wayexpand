# Evdev Access Design

This document records the security direction for WayExpand's raw evdev input
path. It is an investigation plan, not a claim that a tighter access
mechanism is already implemented or portable.

## Current state: legacy/simple access

`--source=evdev` currently requires `scripts/install-evdev-permissions.sh`.
That script installs a udev rule and adds the desktop user to the system
`input` group. The group is a broad grant: a process running as that user can
read raw input devices, including keyboards outside the active Wayland session
and password prompts.

This remains an explicit, administrator-approved fallback for compositors that
do not provide a usable input-method or other capture path. It is not the
long-term preferred security architecture. Automatic backend selection never
enables evdev merely because the current process can read `/dev/input`.

Before using it, administrators should review [SECURITY.md](../SECURITY.md),
run the permission script with `--dry-run`, and document the grant. Remove it
with `--uninstall` when the evdev deployment is retired.

## Candidate tighter mechanisms

| Candidate | Potential improvement | Risks and unknowns |
| --- | --- | --- |
| logind/active-seat ACLs (`uaccess`) | Limits device access to the active local seat instead of every session/user with `input` membership | Requires correct logind/udev integration; behavior across distributions, seat switches, VT changes, suspend/resume, and user services needs real testing |
| Small privileged device broker | Keeps raw device FDs out of the main daemon and can restrict which keyboard devices are opened | Adds a privileged IPC boundary, broker attack surface, FD lifecycle/hotplug complexity, and a new policy/configuration surface |
| Per-device administrator-managed ACLs | Grants only selected keyboard nodes to a selected user | Device names and stable identity vary; hotplug, multiple keyboards, and seat transitions can silently break coverage or broaden access |

No candidate should be enabled by default based on a single successful probe.
In particular, `uaccess` is a promising direction to investigate, not an
automatic replacement for the current group rule.

## Investigation plan

1. Build a read-only prototype for active-seat ACL detection. Do not mutate
   device permissions or group membership during probing.
2. Test logind/udev behavior on representative systemd distributions and on
   sessions with seat changes, VT switches, suspend/resume, fast user switching,
   hotplugged keyboards, multiple keyboards, and no active seat.
3. Specify the broker boundary only if ACLs cannot provide reliable coverage:
   allowed device identity, peer authentication, FD passing, hotplug handling,
   daemon restart behavior, and failure-closed semantics.
4. Run the same tests with two users and a password prompt. Confirm that an
   unprivileged WayExpand daemon cannot read devices outside its approved seat
   or device set.
5. Keep the input-group path available as an explicitly named legacy fallback
   until a replacement is verified and documented for the supported platforms.

## Acceptance criteria for a future replacement

A tighter mechanism is not ready for default use until it demonstrates:

- no permanent `input` group membership is required;
- access is restricted to the intended active seat and approved keyboard FDs;
- access is revoked or becomes unusable after seat/session ownership changes;
- keyboard hotplug and compositor restart behavior is bounded and observable;
- the main daemon remains unprivileged and any privileged component has a
  minimal, auditable interface;
- denial and failure modes are fail-closed, with actionable `doctor` output;
- behavior is reproduced on more than one distribution and desktop session.

Until then, choose between the explicit evdev tradeoff and a compositor/source
that does not require raw kernel keyboard access.
