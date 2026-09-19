# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Summary

WayExpand is a Wayland-native text expansion tool (like Espanso or AutoKey) written in Rust. It's designed with a strict separation between:
- **Matching engine** (core): Unicode-safe trie matcher, templates, undo, config validation
- **Input sources** (capture): input-method-v2, evdev, kwin-window
- **Output backends** (injection): libei/EIS, wlroots virtual-keyboard

Backends are pluggable traits, explicitly selected at runtime—not auto-detected. This design ensures compositors with missing protocol support are documented gaps, not silent failures. The daemon is non-systemd-dependent and runs per-session.

**Current status:** 26/39 issues resolved (67% complete). Core engine is production-quality. Five P0 blockers remain before v1.2 tag (see `REMAINING_P0_BLOCKERS.md` for specs).

---

## Build & Development

### Prerequisites
```bash
# Ubuntu/Debian
sudo apt-get install libwayland-dev libxkbcommon-dev pkg-config

# Arch
sudo pacman -S wayland libxkbcommon pkg-config
```

### Common Commands

**Build:**
```bash
cargo build --locked --workspace              # Debug, all crates
cargo build --locked --release --workspace    # Release binaries
cargo build --locked -p wayexpand-daemon      # Single crate only
```

**Test:**
```bash
cargo test --locked --workspace               # All unit tests (debug)
cargo test --locked --release --workspace     # All unit tests (optimized)
cargo test --locked -p wayexpand-core         # Single crate
cargo test --locked -- --nocapture            # With stdout
```

**Lint & Format:**
```bash
cargo fmt --all -- --check                    # Check formatting
cargo fmt --all                               # Apply formatting
cargo clippy --locked --workspace --all-targets -- -D warnings  # Both debug & release
```

**Integration tests:**
```bash
bash scripts/smoke-daemon.sh                  # Daemon startup test
bash scripts/test-doctor.sh                   # Diagnostics test
bash scripts/test-ui.sh                       # TUI smoke test
bash scripts/test-release.sh                  # CLI smoke test
```

**Run locally:**
```bash
cargo run --locked -p wayexpand-cli -- doctor          # Diagnostics
cargo run --locked -p wayexpand-daemon -- --help       # Daemon help
cargo run --locked -p wayexpand-ui -- --help           # TUI help
cargo run --locked -p wayexpand-gui -- --help          # GUI help
```

---

## Architecture Overview

```
                    ┌─────────────────────────────┐
                    │   wayexpand-core (engine)   │
                    │  trie matcher · templates   │
                    │  config validation · undo   │
                    └───────────┬─────────────────┘
                                │  TextInjector / InputSource traits
                ┌───────────────┼──────────────────────────┐
                │               │                          │
      ┌─────────▼──────┐ ┌──────▼──────────┐   ┌──────────▼─────────┐
      │ input-method-v2│ │ evdev capture   │   │   libei / EIS      │
      │  (capture+type)│ │(compositor-     │   │ (portal-mediated)  │
      │ (exclusive,    │ │ agnostic, needs │   │     output)        │
      │  XKB-based)    │ │ `input` group)  │   └────────────────────┘
      └────────────────┘ └─────────────────┘
                                │
                    ┌───────────▼──────────────┐
                    │ wlroots virtual-keyboard│
                    │    (output only)         │
                    └──────────────────────────┘
```

### Crate Structure

| Crate | Role |
|-------|------|
| **core** | Engine: trie matcher, expansion logic, config validation, undo. No backend deps. Contains ~91 unit tests. |
| **daemon** | Long-running service. Loads backends dynamically, manages config reloads, control socket, event loop. ~53KB main.rs (off-thread injection added 2026-09-18). |
| **cli** | CLI commands (`test`, `preview`, `list`, `doctor`, `validate`, etc.) + JSON/human output formatting. Single-file implementation. |
| **ui** | Terminal UI (TUI) built with Crossterm. Snapshot caching, live preview, app filtering support. |
| **gui** | GUI built with egui. Dashboard, search, diagnostics, import, atomicity. Color packs (VT220, 3270, C64, etc.). |
| **backend-* (6 crates)** | Modular backend implementations. Each implements `InputSource` and/or `TextInjector` traits. See [docs/BACKENDS.md](docs/BACKENDS.md). |

### Key Design Principles

1. **Explicit backend selection:** Daemon requires `--source=` and `--backend=` flags. No auto-detection. This makes gaps discoverable.
2. **Fail-closed for safety:** `app_filter` snippet without window tracker = no match (not match-everywhere). `word-boundary` mode without context = no match.
3. **Trait-based backends:** New backends don't require daemon changes. `Box<dyn InputSource>` and `Box<dyn TextInjector>` keep engine agnostic.
4. **Atomic config reloads:** Old config stays live until new one validates. Malformed config never replaces working one.
5. **Bounded resources:** Every queue, child process, and buffer has a limit. No OOM from config mistakes.

---

## Important Files & Modules

### Core Engine (`crates/core/src/`)

| File | Purpose |
|------|---------|
| `engine.rs` (71KB) | `ExpansionEngine`: trie matching, template vars, command execution, undo, hotkeys, app_filter evaluation. Core business logic. |
| `config.rs` (41KB) | `Config` struct + validation. TOML parsing, schema validation, atomic save-on-replace. |
| `matcher.rs` | Suffix-matching logic. Must preserve longest-match semantics. |
| `template.rs` | Variable expansion: `{{date}}`, `{{username}}`, `{{cursor}}`, command fallback. No shelling out. |
| `backend.rs` | Trait definitions + environment probing (`discover_backends`). Backend discovery is environment-based only (WAYLAND_DISPLAY, /dev/input, etc.), no runtime negotiation. |
| `keys.rs` | Hotkey chord parsing + normalization (e.g., "ctrl+shift+a" → canonical form). |

### Daemon (`crates/daemon/src/`)

| File | Purpose |
|------|---------|
| `main.rs` (53KB) | Event loop, backend loading, input processing. Recent changes (2026-09-18): off-thread injection for slow backends (libei keysym fallback sleeps 12ms/char). New functions: `process_event_offthread`, `inject_results_offthread`, `NullInjector`. |
| `control.rs` | Control socket protocol parser (`status`, `reload`, `pause`, `resume`, `stop`). One-command-at-a-time semantics. |
| `reload.rs` | Config file monitoring, atomic reload, diff calculation for audit logs. |

### CLI (`crates/cli/src/`)

| File | Purpose |
|------|---------|
| `main.rs` (945 lines) | Single-file CLI. `doctor` command at line 472 — **this is where P0 blocker #15 needs fixing** (combine source+backend diagnostics, not just individual probes). |

### Frontends (`crates/ui/src/`, `crates/gui/src/`)

- **TUI** (`ui`): Snapshot caching for visible expansions. Recent cache invalidation fix (2026-09-18): clear both `visible_cache` and `preview_cache` on config change.
- **GUI** (`gui`): egui-based, color theme support, app context selector needed for #20 (preview app-filtered snippets).

---

## Key Development Workflows

### Adding a New Input Source Backend

1. Create `crates/backend-{name}/src/lib.rs`
2. Implement `InputSource` trait from `wayexpand_core::backend`
3. Update `Cargo.toml` workspace members
4. Daemon loads it via `Box<dyn InputSource>` if `--source={name}` passed
5. Update [docs/BACKENDS.md](docs/BACKENDS.md) with availability matrix

**Example:** `backend-input-method` is ~200 lines (protocol setup) + event loop. No engine changes needed.

### Adding Template Variable

1. Edit `crates/core/src/template.rs`: extend `render()` pattern matching
2. Add test case in same file
3. No engine or daemon changes needed—template module is self-contained
4. Update [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) if it's a stable contract

### Fixing a Bug in Matching

1. Add failing test to `crates/core/src/engine.rs::tests` or `matcher.rs::tests`
2. Fix logic
3. Verify `cargo test --locked --workspace` passes
4. Verify `cargo clippy --locked --workspace --all-targets -- -D warnings` passes
5. Commit with message like `fix: <specific issue>` and reference issue #N if applicable

### Running Specific Tests

```bash
# Run a single test
cargo test --locked -p wayexpand-core -- --exact test_name --nocapture

# Run all tests in a module
cargo test --locked -p wayexpand-core engine::tests -- --nocapture

# Run release-mode tests (catches optimization bugs)
cargo test --locked --release -p wayexpand-daemon
```

---

## P0 Blockers & Release Status

**Current state:** 26/39 issues resolved. 5 P0 blockers hold the v1.2 tag.

### P0 Blockers (Estimated 2-3 weeks total)

| # | Issue | Impact | Effort | Status |
|---|-------|--------|--------|--------|
| **15** | Fix `wayexpand doctor` | KDE users blocked; doctor doesn't recognize evdev+libei as valid | 2-3d | ❌ REMAINING |
| **16** | Mark GitHub /releases/latest | Project appears unmaintained despite active dev | 5min | ❌ REMAINING |
| **17-18** | Separate diagnostics dimensions | Confuse implementation with permission status | 3-4d | ❌ REMAINING |
| **19** | Fix app-filter matching | **SECURITY**: window title can override app_id | 2-3d | ❌ REMAINING |
| **20** | App-filter preview context | User sees false "no match" in preview | 3-4d | ❌ REMAINING |

See [docs/REMAINING_P0_BLOCKERS.md](docs/REMAINING_P0_BLOCKERS.md) for full specs, test cases, and implementation guidance.

### P1 Issues (Address in v1.2.1)

- **#25:** Async command execution (blocks input thread, 5+ sec latency)
- **#27:** Process group timeout (kills only immediate child, not descendants)

---

## Testing & CI

### Local CI Simulation

Run the exact jobs from `.github/workflows/ci.yml`:

```bash
# Supply chain security audit
cargo audit

# Rust tests & linting (the main job)
cargo fmt --all -- --check
cargo test --locked --workspace
cargo test --locked --release --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings

# Script validation
shellcheck scripts/*.sh

# Integration tests
bash scripts/smoke-daemon.sh
bash scripts/test-doctor.sh
bash scripts/test-ui.sh
bash scripts/test-install-user.sh

# Systemd unit validation
systemd-analyze verify systemd/wayexpand.service
systemd-analyze verify systemd/wayexpand-input-method.service
systemd-analyze security --offline=yes systemd/wayexpand.service
```

### Contract Tests

- `crates/cli/src/main.rs::tests::status_json_matches_documented_stable_contract` — Ensures `wayexpand status --json` doesn't drift from [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)
- Adding new stable contracts? Add a test in the same module.

---

## Documentation Conventions

- **COMPATIBILITY.md:** Stable JSON schemas, CLI exit codes, status field contracts. Changes here are breaking if they remove fields or change types.
- **BACKENDS.md:** Capability matrix per backend (capture source, output capability, protocol, known limitations).
- **REMAINING_P0_BLOCKERS.md:** Implementation specs with test cases. Referenced from release planning.
- **SECURITY.md:** Threat model, sensitive-field handling per backend, socket permissions.

If a fix touches user-visible behavior, update the relevant doc.

---

## Common Pitfalls & Patterns

1. **Cargo.lock committed:** Always use `--locked` in CI and local builds to match CI environment exactly.
2. **Wayland-only code:** Always gate with `WAYLAND_DISPLAY` checks. Non-Wayland environments (CI, headless) must not crash.
3. **Backend symmetry:** If adding an `InputSource` backend, consider whether `TextInjector` output is also needed (usually evdev input pairs with libei/wlroots output).
4. **Config atomicity:** Config changes always go through `Config::validate()` before `save_atomic()`. Never hand-edit and hope.
5. **Undo state:** Undo only stores the last expansion. Multi-undo not supported. Undo is best-effort and clears on config reload.

---

## Debugging Tips

### Daemon Logs

```bash
# Enable debug logging
RUST_LOG=debug wayexpand-daemon --source=input-method --backend=libei

# Specific module
RUST_LOG=wayexpand_core::engine=debug,wayexpand_daemon=info wayexpand-daemon
```

### Doctor Output

```bash
wayexpand doctor           # Human-readable diagnostics
wayexpand doctor --json    # Structured for scripts/health checks
```

Tells you:
- Which backends are implemented vs. unavailable
- Permission issues (e.g., `/dev/input` not readable)
- Socket availability
- Config validity

### Profiling

```bash
# Flamegraph (requires perf)
sudo perf record -g wayexpand-daemon --source=input-method --backend=libei
perf script > out.perf
cargo install flamegraph
flamegraph --perf out.perf
```

The recent off-thread injection fix (2026-09-18) was specifically to avoid blocking the main loop during slow libei fallback keysym injection (12ms per character). If you see latency, check if the injector is blocking the event loop.

---

## Versioning & Releases

- Workspace version in `Cargo.toml`: `[workspace.package] version = "1.1.2"`
- Version must match across Cargo.toml, debian/changelog, PKGBUILD, RPM spec before release
- `scripts/prepare-release.sh` automates version bumps
- See [docs/RELEASING.md](docs/RELEASING.md) for full release procedure

---

## Resources

- **Main README:** [README.md](README.md) — Quick start, feature table, comparison to Espanso/AutoKey
- **Compatibility contract:** [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) — Stable CLI & JSON guarantees
- **Backend guide:** [docs/BACKENDS.md](docs/BACKENDS.md) — Which protocols each backend uses, limitations
- **Security model:** [SECURITY.md](SECURITY.md) — Threat model, permission handling, sensitive fields
- **Sysadmin guide:** [docs/FOR_SYSADMINS.md](docs/FOR_SYSADMINS.md) — Deployment, monitoring, multi-user
- **Blockers guide:** [docs/REMAINING_P0_BLOCKERS.md](docs/REMAINING_P0_BLOCKERS.md) — v1.2 implementation specs
