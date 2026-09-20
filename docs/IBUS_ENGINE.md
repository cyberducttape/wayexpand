# IBus engine integration

`wayexpand-backend-ibus` contains the native-engine boundary used by an IBus
service. It feeds IBus keysyms into the same `ExpansionEngine` used by the
Wayland daemon and returns protocol-neutral actions:

- ordinary printable keys are committed immediately, so they are not delivered
  twice by the toolkit;
- a match emits `DeleteSurroundingText` followed by `CommitText`;
- a word-boundary delimiter is included in that one replacement operation;
- shortcuts, navigation keys, backspace, and disabled focus are forwarded to
  the client and clear matcher state.

The `wayexpand-ibus` binary now supplies that service wrapper. It claims a
private IBus engine name, exposes the standard factory/engine objects, and
translates actions into `CommitText`/`DeleteSurroundingText` signals. The
adapter remains separate so matching and deletion semantics stay testable
without an IBus session.

`app_filter` entries fail closed in this mode because IBus does not provide a
portable focused-window identity. IBus content-purpose values for password and
PIN fields disable matching; other toolkit-specific sensitivity hints are not
currently interpreted.

After installing, make sure `~/.local/bin` is on `PATH`, then restart IBus and
select `WayExpand` (`wayexpand` engine) in the desktop input-method settings.
