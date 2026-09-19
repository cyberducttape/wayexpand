# Security model

WayExpand is designed for a single unprivileged desktop user. It must not be
run as root and should not be granted broader filesystem or device permissions
than the selected backend requires.

## Trust boundaries

| Boundary | Protection |
| --- | --- |
| Configuration file | Regular-file check, trusted owner, no group/other write, bounded size. |
| Configuration ancestors | Owner and writable-mode validation, symlink resolution before load. |
| Daemon control socket | User-owned, mode 0600, validated parent/ancestors, stale identity checks. |
| Input stream | Bounded lines/queues; invalid UTF-8 and oversized data are discarded. |
| Text injection | Explicit backend, bounded output, fail-closed errors, no unsafe replay. |
| Sensitive focus | Clears matcher state and disables matching until focus is safe. |
| Command expansion | Direct process execution, bounded args/output/time, no shell. |

## Sensitive fields

The input-method source starts disabled and enables capture only after receiving
a non-sensitive content purpose. Password, hidden-text, sensitive-data, and
unknown values are treated as sensitive. A sensitive focus event clears the
rolling buffer immediately.

## Backend permissions

- input-method-v2 requires compositor support and a valid Wayland session.
- wlroots virtual-keyboard requires the compositor protocol and permission to
  create a virtual keyboard.
- libei/EIS requires explicit backend selection and a configured/authorized
  session as described in [SECURITY.md](../../SECURITY.md).

Run `wayexpand doctor` after changing desktop permissions. Avoid “fixes” that
make the configuration directory world-writable or run the daemon as root.
