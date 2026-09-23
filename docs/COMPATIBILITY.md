# Compatibility & Stability Guarantees

This document defines the stability guarantees for WayExpand 1.x. Third-party tools, automation, and integrations can rely on these surfaces remaining stable across minor versions.

**Stability levels:**
- 🔒 **Stable** — guaranteed backward-compatible within 1.x; breaking changes require major version
- 🟡 **Experimental** — may change between 1.x releases; changes documented in release notes
- ⚠️ **Unstable** — subject to change without notice; not intended for external consumption

---

## CLI Exit Codes

All WayExpand commands use standardized exit codes, enabling reliable automation and service monitoring.

| Code | Category | Meaning | Examples |
|------|----------|---------|----------|
| **0** | Success | Command completed successfully | Any successful command |
| **1** | Generic Error | I/O, network, or other transient failure | File not found, socket timeout |
| **2** | Usage Error | Invalid command or arguments | Unknown command, missing required arg, malformed hotkey chord |
| **3** | Configuration Error | Invalid or malformed configuration | Syntax error in TOML, invalid expansion trigger, missing required field |
| **4** | Daemon Error | Daemon communication failure | Cannot connect to control socket, daemon unresponsive, non-UTF-8 response |

**Stability:** 🔒 **Stable** — guaranteed stable across 1.x

**Usage example (CI/monitoring):**
```bash
#!/bin/bash
wayexpand validate "$CONFIG_PATH"
case $? in
  0) echo "Config OK" ;;
  3) echo "Config invalid (exit 3)" >&2; exit 1 ;;
  *) echo "Unexpected error" >&2; exit 1 ;;
esac
```

---

## JSON Output Formats

Commands with `--json` flag produce structured output suitable for parsing by external tools. Each command's JSON shape is stable within 1.x.

### `wayexpand test --json [config]`

Tests a text input against the expansion engine and returns matching results.

**Output shape:**
```json
{
  "matched": true,
  "results": [
    {
      "trigger_characters": 5,
      "erase_characters": 5,
      "replacement_bytes": 23,
      "replacement": "example@example.com",
      "cursor_offset": null
    }
  ]
}
```

**Field stability:**
- `matched` (bool): Always present; true if one or more results
- `results` (array): Always present; empty if no matches
  - `trigger_characters` (int): Number of characters in trigger (Unicode grapheme count)
  - `erase_characters` (int): Number of characters to erase (may differ from trigger for multi-char sequences)
  - `replacement_bytes` (int): Byte length of replacement (for buffer sizing)
  - `replacement` (string): The expanded text to insert
  - `cursor_offset` (int or null, 1.0.1+): Characters to move the cursor left after typing `replacement`, from a `{{cursor}}` marker in the source template; `null` when the replacement has no marker (cursor stays at the end, matching every replacement written before this field existed)

**Stability:** 🔒 **Stable** — fields guaranteed present; new fields added after existing ones with opt-in via consumer code

---

### `wayexpand test-hotkey --json <chord> [config]`

Tests a key chord against hotkey triggers without executing actions.

**Output shape:**
```json
{
  "chord": "Ctrl+Alt+M",
  "matched": true,
  "actions": [
    {
      "description": "Insert daily standup template",
      "program": "/usr/bin/true",
      "args": ["template", "--type=standup"]
    }
  ]
}
```

**Field stability:**
- `chord` (string): Canonical representation of the key chord
- `matched` (bool): True if one or more actions match
- `actions` (array): Array of matching hotkey actions (empty if no matches)
  - `description` (string): Human-readable description of the hotkey
  - `program` (string): Executable program name (first component of command)
  - `args` (array of strings): Command arguments (does not include program name)

**Stability:** 🔒 **Stable** — guaranteed present; new fields added after existing

---

### `wayexpand preview --json <trigger> [config]`

Shows what an expansion would produce without actually expanding.

**Output shape (matched):**
```json
{
  "matched": true,
  "trigger": ";email",
  "replacement": "user@example.com",
  "cursor_offset": null
}
```

**Output shape (no match):**
```json
{
  "matched": false
}
```

**Field stability:**
- `matched` (bool): Always present
- `trigger` (string): Present only if `matched: true`
- `replacement` (string): Present only if `matched: true`
- `cursor_offset` (int or null): Present only if `matched: true`. Characters
  from the end of `replacement` where the cursor should land, for a
  template containing `{{cursor}}`; `null` if the template has no cursor
  marker.

**Stability:** 🔒 **Stable** — presence of optional fields conditional on `matched`

---

### `wayexpand list --json [config]`

Lists all configured expansions and hotkeys.

**Output shape:**
```json
{
  "config": "/home/user/.config/wayexpand/expansions.toml",
  "count": 12,
  "hotkey_count": 3,
  "expansions": [
    {
      "trigger": ";email",
      "replacement": "user@example.com",
      "description": "Personal email",
      "enabled": true,
      "tags": ["contact"],
      "match_mode": "word-boundary",
      "app_filter": [],
      "command": null,
      "propagate_case": false,
      "category": "Personal"
    }
  ],
  "hotkeys": [
    {
      "chord": "Ctrl+Alt+M",
      "description": "Standup template",
      "command": {
        "program": "/usr/bin/cat",
        "args": ["/home/user/templates/standup"],
        "timeout_ms": 5000,
        "cache_ms": 0
      },
      "enabled": true
    }
  ]
}
```

**Field stability:**
- `config` (string): Path to configuration file
- `count` (int): Number of expansions
- `hotkey_count` (int): Number of hotkeys
- `expansions` (array): Full expansion configurations (see TOML schema below)
- `hotkeys` (array): Full hotkey configurations

**Note:** `expansions` and `hotkeys` contain the complete configuration objects; their schema is covered by TOML config stability guarantees.

**Stability:** 🔒 **Stable** — top-level counts and paths stable; contents follow config schema stability

---

### `wayexpand search --json <query> [config]`

Searches expansions by trigger, description, and tags.

**Output shape:**
```json
{
  "query": "email",
  "config": "/home/user/.config/wayexpand/expansions.toml",
  "count": 2,
  "expansions": [
    {
      "trigger": ";email",
      "replacement": "user@example.com",
      "description": "Personal email",
      "enabled": true,
      "tags": ["contact"],
      "match_mode": "word-boundary",
      "app_filter": [],
      "command": null,
      "propagate_case": false,
      "category": "Personal"
    }
  ]
}
```

**Field stability:**
- `query` (string): The search query (echoed back)
- `config` (string): Path to configuration file
- `count` (int): Number of matching expansions
- `expansions` (array): Matching expansion objects

**Stability:** 🔒 **Stable** — same as `list` output

---

### `wayexpand doctor --json [config]`

Diagnostic output suitable for health checks and monitoring systems.

**Output shape:**
```json
{
  "healthy": true,
  "wayland": true,
  "config": {
    "path": "/home/user/.config/wayexpand/expansions.toml",
    "valid": true,
    "error": null
  },
  "control_socket": {
    "path": "/run/user/1000/wayexpand.sock",
    "configured": true,
    "exists": true
  },
  "policy": {
    "path": "/etc/wayexpand/policy.toml",
    "exists": false,
    "policy": { "valid": true, "is_active": false }
  },
  "backends": [
    {
      "kind": "input-method-v2",
      "state": "Implemented",
      "detail": "manager available; seat connection pending"
    },
    {
      "kind": "wlroots-virtual-keyboard",
      "state": "Implemented",
      "detail": "virtual keyboard globals available"
    }
  ],
  "automatic_selection": {
    "source": "stdin",
    "backend": "libei",
    "reason": "conservative default: stdin + libei (readable evdev is available but disabled by default)",
    "ready": false
  },
  "setup_recommendation": {
    "mode": "recommended",
    "backend": "ibus",
    "label": "IBus",
    "detail": "toolkit-aware committed text",
    "ready": true
  },
  "capture_readiness": {
    "state": "available-to-try",
    "detail": "a protocol or IBus probe succeeded; live client typing is not verified",
    "end_to_end_verified": false
  }
}
```

**Field stability:**
- `healthy` (bool): Overall health (an active display session, config and organization policy valid, a usable automatic input path or installed IBus path, and socket operational if configured)
- `wayland` (bool): Wayland session detected
- `config.path` (string): Configuration file path
- `config.valid` (bool): Configuration syntax valid
- `config.error` (string|null): Error message if invalid (safe summary, not a stack trace)
- `control_socket.path` (string|null): Socket path (null if WAYEXPAND_SOCKET and XDG_RUNTIME_DIR both unset)
- `control_socket.configured` (bool): Socket path available (either env var or XDG_RUNTIME_DIR)
- `control_socket.exists` (bool): Socket file exists on filesystem
- `control_socket.valid` (bool): Existing path is a user-owned, non-group/world-accessible Unix socket
- `policy` (object): Organization-policy validation result from the same secure loader used by the daemon
- `backends` (array): Available backends
  - `kind` (string): One of "input-method-v2", "evdev", "libei", "wlroots-virtual-keyboard", "uinput", "clipboard", "window-tracker"
  - `state` (string): One of "Implemented", "Available", "Unavailable", "NotImplemented", "RequiresPermission" (the `BackendState` enum in `crates/core/src/backend.rs`; "Available" is defined but no backend reports it today). These legacy values mix implementation status with environment status; consumers should use the separate fields when present.
  - `implementation` (string): One of "Implemented" or "NotImplemented".
  - `availability` (string): One of "Detected", "Unavailable", or "Unknown".
  - `permission` (string): One of "Granted", "Required", or "NotApplicable".
  - `policy_allowed` (bool|null): Whether the shared organization policy
    permits this selectable backend; null for input sources, trackers, and
    unshipped entries that are not directly governed as output backends.
  - `detail` (string): Human-readable details (e.g., reason for unavailability)
- `automatic_selection` (object): The daemon's shared source/backend resolver result; `ready` is false for the conservative stdin-only fallback.
- `setup_recommendation` (object): The setup mode selected from current capabilities; it never turns an experimental path into Recommended mode.
- `capture_readiness` (object): Non-invasive source/output readiness after organization-policy filtering across IBus, libei, wlroots, and input-method-v2. `state` is one of `available-to-try`, `authorization-required`, `not-probed`, or `unavailable`; `end_to_end_verified` remains false until a compositor/client certification harness supplies evidence.
- `capabilities` (array): Backend feature contracts, including explicit `limitations` that consumers must display rather than infer away.

The graphical diagnostics view uses the same IBus installation probe as the
CLI and labels protocol implementations as warnings until their runtime
availability is detected; an implemented backend is not displayed as a
certified or ready path. It also uses the shared secure organization-policy
loader and explicitly reports an invalid policy as a daemon-startup blocker.

**Stability:** 🔒 **Stable** — guaranteed to include `healthy`, `config`, `control_socket`, `backends`; new backend states may be added

**Usage example (systemd health check):**
```bash
#!/bin/bash
# systemd ExecStartPost health check
wayexpand doctor --json | jq -e '.healthy' >/dev/null || exit 1
```

---

### `wayexpand status --json`

Current daemon status (requires running daemon). This wraps the daemon's
plain-text control-socket status response (`response` is its first line;
every subsequent `key=value` line becomes a field, with `true`/`false`
parsed as JSON booleans and nonnegative integer values parsed as JSON numbers)
rather than a purpose-built schema, so the fields
below are exactly what that response currently carries -- nothing more.

**Output shape:**
```json
{
  "response": "running",
  "source": "input-method",
  "backend": "input-method-v2",
  "state": "connected",
  "paused": false,
  "config": "/home/user/.config/wayexpand/expansions.toml",
  "config_state": "ok",
  "command_queue_depth": 0,
  "command_queue_rejected_total": 0,
  "command_timeout_total": 0,
  "command_failure_total": 0
}
```

**Field stability:**
- `response` (string): The control socket's status line, e.g. `"running"`
- `source` (string): Active input source (`"input-method"`, `"stdin"`, `"evdev"`, ...)
- `backend` (string): Active output backend (`"input-method-v2"`, `"wlroots-virtual-keyboard"`, `"libei"`, ...)
- `state` (string): Backend connection state (e.g. `"connected"`, `"reconnecting"`)
- `paused` (bool): Whether expansion matching is currently disabled
- `config` (string): Path to the active configuration file
- `config_state` (string): `"ok"` or `"reload-rejected"` (the daemon kept its previous configuration because the last reload was invalid)
- `command_queue_depth` (integer): Number of accepted command actions waiting for the command worker
- `command_queue_rejected_total` (integer): Number of command actions rejected because the bounded queue was full or unavailable
- `command_timeout_total` (integer): Number of command actions that exceeded their configured timeout
- `command_failure_total` (integer): Number of command actions that failed for another reason, including spawn failures and non-zero exits

**Stability:** 🔒 **Stable** — these fields are guaranteed; new fields may be
added. The queue depth is instantaneous; the three totals cover the current
daemon engine and reset after a successful configuration reload.

---

## TOML Configuration Schema Stability

The configuration file format (`~/.config/wayexpand/expansions.toml`) is stable within 1.x.

### Schema Guarantees

**Existing fields** (0.1.0+):
- `[[expansion]]` section with `trigger`, `replacement`, `description`, `enabled`
- `[[hotkey]]` section with `chord`, `description`, `command`
- Global `[settings]` section with `max_buffer_chars`

These fields are guaranteed present and backward-compatible. Missing fields use `#[serde(default)]` to provide sensible defaults.

**New fields** (0.2.0+):
- `ExpansionConfig::category` — optional categorization
- `ExpansionConfig::app_filter` — optional app-scoped restrictions (Vec<String>)
- `ExpansionConfig::tags` — optional searchable tags
- `ExpansionConfig::match_mode` — expansion matching mode ("immediate" or "word-boundary", default "immediate")

**New fields** (1.0.1+):
- `ExpansionConfig::propagate_case` (bool, default `false`) — when enabled,
  typing the trigger in `UPPERCASE` or `Capitalized` form applies the same
  casing to the replacement (e.g. trigger `:sig` typed as `:SIG` yields an
  uppercased replacement). Off by default, so existing configs are
  unaffected; matching stays strictly literal unless a snippet opts in.
- `Settings::undo_chord` (string or absent, default absent) — a key chord
  (e.g. `"Ctrl+Z"`) that, pressed immediately after a successful expansion
  with no other keystroke in between, reverts it. Disabled unless set.
- Template variables `date`, `time`, and `datetime` accept a relative
  offset (e.g. `{{date+3d}}`, `{{time-2h}}`); see archived
  [Configuration.md](archive/wiki/Configuration.md) for examples.
- `{{cursor}}` template marker — places the cursor at that position after
  the replacement is typed, instead of at the end. Supported on the libei
  and wlroots backends; silently has no effect on input-method-v2 (no
  protocol-level way to move the cursor after committing text).

All new fields default to `null`, `false`, or empty if absent from old configurations.

**Backward compatibility guarantee:**
```toml
# Old config (0.1.0) still loads in 1.x
[[expansion]]
trigger = ":email"
replacement = "user@example.com"
```

**Forward compatibility guarantee:**
A config file written by 1.x continues to load in future minor versions (1.1, 1.2, etc.):
- New fields are added with `#[serde(default)]` or explicit default values
- Existing fields retain their meaning and type
- Fields are never removed within 1.x

**Stability:** 🔒 **Stable** — guaranteed backward and forward compatible within 1.x

---

## Breaking Changes & Major Version Bumps

To make a **breaking change** that violates the above guarantees, a new major version is required (2.0.0).

Examples of breaking changes:
- Changing the type of an existing field (e.g., `enabled: bool` → `enabled: string`)
- Removing a field or CLI command
- Changing the meaning of an exit code
- Removing fields from JSON output

Breaking changes **must**:
1. Be documented prominently in release notes
2. Be accompanied by a migration guide
3. Bump to major version (X.0.0)
4. Provide tooling or clear instructions for config migration if applicable

---

## Configuration Migration & Future Changes

**For hypothetical breaking changes in future major versions:**

1. **At-risk period:** Users of version N.x will have until version (N+1).0 to migrate
2. **Migration path:** 
   - Release notes include step-by-step migration instructions
   - Automated migration tool provided if applicable (e.g., `wayexpand migrate-config <old> <new>`)
   - Old format continues to load if possible (with deprecation warning)
3. **Testing:** Test suite includes regression tests for old config format (like `config::tests::pre_category_config_without_new_fields_still_parses`)

---

## Dependency Stability

**Runtime dependencies:**
- Wayland protocol implementations (input-method-v2, wlroots, libei, EIS)
- systemd user services (for daemon auto-start)
- D-Bus session service (for KDE Plasma window tracking, when available)

**Build dependencies:**
- Rust 1.87+ (the workspace MSRV declared in `Cargo.toml` and `rust-toolchain.toml`)
- Standard development libraries (libwayland-dev, libxkbcommon-dev, pkg-config)

These are not guaranteed stable across 1.x (new systemd versions, wayland protocol updates), but breaking changes are flagged with upstream deprecation warnings. Distro package maintainers should test before publishing major distro updates.

**Stability:** 🟡 **Experimental** — external dependencies subject to upstream changes; WayExpand follows upstream semver

---

## What's NOT Stable

The following surfaces are **not** guaranteed stable and may change between minor versions:

- **Internal crate API** (non-public re-exports, internal modules)
- **Daemon control protocol** (socket communication format) — may be extended but not changed
- **Config reload behavior** (exact timing of propagation, error handling edge cases)
- **Performance characteristics** (matching algorithm, memory usage)
- **Error messages** (may be reworded for clarity; do not parse error text)
- **Diagnostic output** (human-readable `wayexpand doctor` text output without `--json`)

---

## Testing & Validation

All stable surfaces are covered by regression tests in the test suite:

- Exit codes verified per command (see `crates/cli/src/main.rs` exit_code_for)
- JSON output shapes validated (see serde json! macros throughout CLI)
- Config backward compatibility tested (see config::tests::pre_category_config_without_new_fields_still_parses)

Configuration resource limits are maintained in
[`CONFIGURATION_LIMITS.md`](CONFIGURATION_LIMITS.md).

The machine-readable contract declarations are versioned in
`tests/contracts/`. Changes to stable JSON field names or types must update a
fixture deliberately and include a compatibility note.

Developers adding new features must maintain these stability guarantees or bump to the next major version.

---

## Questions?

For clarification on what's stable for your use case, open an issue on GitHub with your integration scenario.
