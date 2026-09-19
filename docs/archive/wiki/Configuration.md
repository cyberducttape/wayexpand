# Configuration reference

The default file is `$XDG_CONFIG_HOME/wayexpand/expansions.toml`, or
`~/.config/wayexpand/expansions.toml` when `XDG_CONFIG_HOME` is unset. Set
`WAYEXPAND_CONFIG` to select another file.

## Complete schema

```toml
[settings]
max_buffer_chars = 128

[[expansion]]
trigger = ":today"
replacement = "Today is {{date}} ({{time}} UTC)."
description = "Current UTC date and time"
tags = ["date", "time"]
match_mode = "immediate" # or "word-boundary"
enabled = true

[[expansion]]
trigger = ":kernel"
replacement = ""
description = "Running kernel release"

[expansion.command]
program = "uname"
args = ["-r"]
timeout_ms = 500
cache_ms = 60000
environment = "minimal" # or "inherit" (explicit opt-in)
pass_env = ["KUBECONFIG"]

[[hotkey]]
chord = "Ctrl+Alt+M"
description = "Open meeting helper"
enabled = true

[hotkey.command]
program = "wayexpand-meeting-helper"
args = []
timeout_ms = 1000
```

The authoritative resource and validation limits are documented in
[`CONFIGURATION_LIMITS.md`](../CONFIGURATION_LIMITS.md).

## Field behavior and limits

| Field | Behavior |
| --- | --- |
| `trigger` | Required, non-empty, NUL-free, maximum 128 Unicode characters. |
| `replacement` | Required unless `command` is set; NUL-free, maximum 1 MiB. |
| `description` | Optional, maximum 512 Unicode characters. |
| `tags` | Optional, maximum 32 tags; each tag is bounded and must be non-empty. |
| `match_mode` | `immediate` expands as soon as a trigger matches; `word-boundary` waits for a safe boundary. |
| `enabled` | Defaults to `true`; disabled entries remain editable but never match. |
| `propagate_case` | Defaults to `false`. When `true`, typing the trigger `UPPERCASE` or `Capitalized` applies the same casing to the replacement. |
| `max_buffer_chars` | Defaults to 128; accepted range is 1–4096. |
| `undo_chord` | Optional key chord (e.g. `"Ctrl+Z"`, same syntax as a hotkey chord). Pressed right after an expansion with nothing typed in between, reverts it. Absent by default (disabled). |
| `command.timeout_ms` | Defaults to 500 ms; accepted range is 1–5000 ms. |
| `command.cache_ms` | Defaults to 0; maximum is 60 seconds. |
| `command.environment` | Defaults to `minimal`; only `HOME`, `USER`, `PATH`, and `LANG` are passed. Set `inherit` only when the command requires the full daemon environment. |
| `command.pass_env` | Optional allowlist of additional variable names copied in minimal mode. Values are read from the daemon environment and never stored in config. |

Hotkeys are normalized across backends. Modifier aliases such as `Control`,
`Option`, `Meta`, `Win`, and `Logo` are accepted and canonicalized to
`Ctrl`, `Alt`, and `Super`. Each enabled chord must be unique. The input-method
backend emits normalized key events and the daemon executes matching actions
without a shell, with bounded arguments and a per-action timeout. Actions are
currently best suited to compositor-safe helper programs; general key
pass-through and additional backends remain active roadmap work.

The entire file is capped at 16 MiB and 10,000 entries. Enabled trigger data
is capped to keep matching memory bounded.

## Templates

Safe built-ins are expanded only after a match:

`{{date}}`, `{{time}}`, `{{datetime}}`, `{{username}}`, `{{hostname}}`,
`{{unix_timestamp}}`, `{{newline}}`, and `{{tab}}`.

`date`, `time`, and `datetime` also accept a relative offset:
`{{date+3d}}` (3 days from now), `{{date-1w}}` (1 week ago),
`{{time+5h}}`, `{{datetime+90m}}`. Supported units are `d` (day), `w`
(week), `h` (hour), and `m` (minute); the sign is required.

`{{cursor}}` places the cursor after the replacement is typed, instead of
leaving it at the end -- useful for a template with text on both sides of
where you'll type next, e.g. `replacement = "(){{cursor}}"` or
`replacement = "Hi {{cursor}},\n\nBest"`. It is supported on the libei and
wlroots virtual-keyboard backends (both can synthesize a Left key after
typing); the input-method-v2 backend has no protocol-level way to move the
cursor after committing text, so the marker is silently stripped there and
the cursor is left at the end as usual. Only the first `{{cursor}}` in a
replacement has effect.

Unknown or unclosed variables fail validation before activation. Templates are
not shell commands.

## Command expansions

Commands execute a configured program directly, with bounded arguments, no
stdin, discarded stderr, a bounded timeout, and a 1 MiB UTF-8 stdout limit.
Shell operators such as `|`, `>`, and `&&` are ordinary arguments, not syntax.
Use command expansions only for trusted local facts; they still run with the
desktop user’s privileges.

## Validation workflow

```sh
wayexpand validate path/to/expansions.toml
wayexpand doctor path/to/expansions.toml
wayexpand list --json path/to/expansions.toml
wayexpand preview ':today' --json path/to/expansions.toml
```

Duplicate triggers, insecure ownership/modes, non-regular files, malformed
TOML, and unsafe command settings are rejected.
