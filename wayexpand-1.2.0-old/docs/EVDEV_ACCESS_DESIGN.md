# Evdev Access Design

This document records the current access modes and remaining security work for
WayExpand's raw evdev input path.

## Current state: active-seat default and legacy/simple fallback

`--source=evdev` requires `scripts/install-evdev-permissions.sh`. The default
`--access=active-seat` mode installs a udev rule for keyboard-class event nodes
and lets systemd-logind manage an ACL for the active local seat. It does not
change permanent group membership. The explicit `--access=input-group` mode
is the legacy fallback: it may add the desktop user to the system `input`
group, which can be broader than WayExpand's keyboard-node rule on a given
distribution.

This remains an explicit, administrator-approved fallback for compositors that
do not provide a usable input-method or other capture path. It is not the
long-term preferred security architecture. Automatic backend selection never
enables evdev merely because the current process can read `/dev/input`.

The default setup uses the seat-aware ACL model:

```sh
sudo ./scripts/install-evdev-permissions.sh --access=active-seat
```

This installs a `TAG+="uaccess"` rule for keyboard-class event nodes and does
not add the user to the broad `input` group. systemd-logind must be active, and
access is granted only while the user owns the active local seat. Verify the
resulting ACL with `getfacl /dev/input/eventN` and confirm that
`wayexpand doctor` can see a keyboard before starting the daemon. The legacy
group model remains available only when explicitly requested with
`--access=input-group`; it has broader, permanent visibility and should be
used only when that tradeoff is explicitly accepted.

Before using either mode, administrators should review [SECURITY.md](../SECURITY.md),
run the permission script with `--dry-run`, and document the grant. Remove it
with `--uninstall` when the evdev deployment is retired.

## Candidate tighter mechanisms

| Mode or candidate | Security/property | Risks and unknowns |
| --- | --- | --- |
| logind/active-seat ACLs (`uaccess`) | Current default; limits device access to the active local seat instead of every session/user with `input` membership | Requires correct logind/udev integration; behavior across distributions, seat switches, VT changes, suspend/resume, and user services needs broader certification |
| Small privileged device broker | Keeps raw device FDs out of the main daemon and can restrict which keyboard devices are opened | Adds a privileged IPC boundary, broker attack surface, FD lifecycle/hotplug complexity, and a new policy/configuration surface |
| Per-device administrator-managed ACLs | Grants only selected keyboard nodes to a selected user | Device names and stable identity vary; hotplug, multiple keyboards, and seat transitions can silently break coverage or broaden access |

No future broker or per-device ACL mechanism should be enabled by default
based on a single successful probe. `uaccess` is already the shipped default;
the remaining investigation concerns broader portability and whether a broker
would provide a useful additional boundary.

## Investigation plan

1. Test the shipped active-seat ACL implementation on representative systemd
   distributions. Do not mutate device permissions or group membership during
   probing.
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
