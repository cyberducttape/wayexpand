# Security model

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

The input-method source fails closed when it cannot safely pass through a
non-text key or determine a UTF-8-safe Backspace range from surrounding text.
Unsupported ordinary non-text keys are discarded individually and clear the
pending matcher state; malformed protocol state remains fatal rather than
risking text corruption.

Backends must document their permission requirements explicitly:

- direct libei/EIS requires an explicitly configured `LIBEI_SOCKET`; portal
  libei requires explicit backend selection and an approved desktop
  remote-desktop session;
- uinput requires device access and emits synthetic keyboard events;
- clipboard fallback can overwrite or expose clipboard contents;
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
