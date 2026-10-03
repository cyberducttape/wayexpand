# IBus engine integration

`wayexpand-backend-ibus` contains the native-engine boundary used by an IBus
service. It feeds IBus keysyms into the same `ExpansionEngine` used by the
Wayland daemon and returns protocol-neutral actions:

- ordinary printable keys are committed immediately, so they are not delivered
  twice by the toolkit;
- a match emits `DeleteSurroundingText` followed by `CommitText`. These are
  two separate signals, so the replacement is **not atomic**: a failure
  between them can leave the trigger deleted without its replacement. The
  route therefore reports `atomic_replace = false`;
- the deletion covers only text the client actually holds: the final key of
  an immediate trigger is consumed and never delivered, so it is not deleted;
- a word-boundary delimiter is included in the replacement commit;
- nothing is deleted unless the client's reported surrounding text confirms
  that the trigger sits immediately before a collapsed cursor (see below);
- shortcuts, navigation keys, backspace, and disabled focus are forwarded to
  the client and clear matcher state.

The `wayexpand-ibus` binary now supplies that service wrapper. It claims a
private IBus engine name, exposes the standard factory/engine objects, and
translates actions into `CommitText`/`DeleteSurroundingText` signals. The
adapter remains separate so matching and deletion semantics stay testable
without an IBus session.

`app_filter` entries fail closed in this mode because IBus does not provide a
portable focused-window identity.

## Sensitive fields

Capture is off until a field positively reports a known, non-sensitive
content type. A new engine, `FocusIn`, `Enable`, `FocusOut`, and `Disable` all
leave capture disabled; ibus-daemon follows every `FocusIn` with
`SetContentType`, and only that call can enable matching. `Reset` does not
change the field's content type. The following are treated as sensitive:

- purposes `PASSWORD` (8) and `PIN` (9);
- any purpose newer than this build knows (greater than `DATETIME`, 13);
- the `PRIVATE` and `HIDDEN_TEXT` input hints, on any purpose.

A client that never sends a content type therefore gets no expansion.

## Surrounding-text verification

The engine requests surrounding text (`RequireSurroundingText`) when a field
gains focus and tracks the client's reported text, cursor, and selection
anchor, advancing that model over the edits it emits itself. A replacement is
emitted only when that text ends with the trigger characters the client holds
and there is no selection. Otherwise the replacement is refused before
anything is deleted and the key is delivered normally. `Reset`, focus changes,
and keys the client applies itself drop the model until the client reports
its text again. Clients without `IBUS_CAP_SURROUNDING_TEXT` get no
replacements.

The IBus service loads the same root-owned `/etc/wayexpand/policy.toml` as the
daemon. Invalid or insecure policy files prevent startup, and active policy
limits (including replacement size and allowed backend) are enforced for IBus
expansions, and capability requirements are checked against the IBus route's
profile: in safe mode, `require_atomic_replace` disables IBus expansion
because the route is not atomic, while `require_sensitive_focus` is satisfied
by content-type reporting. Direct executable commands remain disabled in IBus because the
IBus service is not the hardened `wayexpand.service` command-execution
boundary. Named Action Broker commands are allowed: they cross the broker's
authenticated Unix-socket boundary and fail closed when the broker is
unavailable. The backend is identified as `ibus` for `allowed_backends`
policy checks, separately from `input-method-v2`; a policy that permits both
must list both.

Each IBus `CreateEngine` request owns one engine object and adapter. IBus
releases that object through `Destroy`; the service removes the adapter from
its registry and unregisters the D-Bus object at that point.

Named actions run on the engine's command workers so `ProcessKeyEvent` never
waits for them. When a result is ready the worker wakes a single completion
thread with the owning engine's path; there is no polling. That thread locks
the engine's adapter, discards results that are stale (any key, reset, or
focus change since the action was queued), checks policy and surrounding
text, emits the delete and commit signals, and records the expansion as
applied only after both were emitted.

Key events are converted with xkbcommon's keysym-to-Unicode mapping, so
legacy keysyms (for example `Greek_alpha` or `Cyrillic_a`) and keypad digits
match like explicit Unicode keysyms. Dead keys and Compose are resolved by the
toolkit before IBus sees the result. These paths are unit tested; behaviour
with real clients across layouts is not yet certified.

After installing, make sure `~/.local/bin` is on `PATH`, then restart IBus and
select `WayExpand` (`wayexpand` engine) in the desktop input-method settings.
