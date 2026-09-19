# Development Guide

**Navigation:** [Home](../README.md) > [For Developers](DEVELOPMENT.md)

---

WayExpand is designed for small, reviewable changes with explicit safety properties. The core engine is platform-independent; compositor-specific behavior lives behind backend traits.

## Setting Up Your Environment

**System dependencies** (Debian/Ubuntu):
```sh
sudo apt install libwayland-dev libxkbcommon-dev pkg-config
```

**Clone the repository:**
```sh
git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand
```

## Development Workflow

**Create a feature branch:**
```sh
git checkout -b feature/short-description
```

**Run checks locally before committing:**
```sh
cargo fmt --all
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
shellcheck scripts/*.sh
```

**Behavior-level testing before opening a PR:**
```sh
bash scripts/smoke-daemon.sh       # Smoke test the daemon
bash scripts/test-doctor.sh        # Test diagnostics
bash scripts/test-ui.sh            # Test terminal UI
bash scripts/test-install-user.sh  # Test installer
```

## Architecture Overview

**Core structure:**
- `crates/core` — Matching engine (platform-independent, no backend dependencies)
  - `matcher.rs` — Reversed char trie over bounded rolling buffer
  - `engine.rs` — Expansion matching and execution
  - `config.rs` — Configuration parsing and validation
  - `backend.rs` — Backend trait definitions (`InputSource`, `TextInjector`, `WindowTracker`)

- `crates/daemon` — Event loop and lifecycle
  - `main.rs` — Daemon entry point and systemd integration
  - `control.rs` — Control socket on separate thread
  - `reload.rs` — Config watching and atomic reload

- `crates/cli`, `crates/ui`, `crates/gui` — User interfaces
  - CLI (`wayexpand` binary)
  - TUI (`wayexpand-ui`, crossterm-based)
  - GUI (`wayexpand-gui`, egui-based)

- `crates/backend-*` — Pluggable backends implementing traits
  - `backend-input-method` — input-method-v2 (capture + output)
  - `backend-evdev` — evdev (capture only, needs `input` group)
  - `backend-libei` — libei/EIS (output only, portal-mediated)
  - `backend-wlroots` — wlroots virtual-keyboard (output only)
  - `backend-kwin-window` — KWin window tracking (D-Bus)

**Key principle:** Backend auto-selection is conservative and explainable; use `wayexpand backend select --explain` or explicit `--source`/`--backend` flags.

## Testing Requirements

### Unit tests

**New parsing or matching behavior** needs unit tests:
```rust
#[test]
fn test_case_matching() {
    // Test propagate_case behavior
}
```

**Lifecycle, reload, permissions, socket behavior** need regression coverage at the daemon/core boundary:
```rust
#[test]
fn test_config_reload_preserves_state() {
    // Verify invalid config doesn't replace working one
}
```

**Rule:** Do not add tests that print real trigger or replacement contents to logs (privacy protection).

### Backend tests

For backend changes, cover:
- **Retry behavior** — Protocol reconnection and bounded backoff
- **Timeout handling** — Command execution with max duration
- **Malformed protocol** — Graceful handling of bad server responses
- **Oversized input** — Text injection limits (1 MiB max)
- **Fail-closed semantics** — Backend must refuse unsafe injection

**Rule:** A backend must fail closed when it cannot establish a safe injection contract.

### Integration tests

**Provided scripts** test real scenarios:
```bash
bash scripts/smoke-daemon.sh     # Basic daemon lifecycle
bash scripts/test-doctor.sh      # Diagnostics accuracy
bash scripts/test-ui.sh          # TUI rendering and navigation
bash scripts/test-install-user.sh  # User-level installer idempotence
```

These run in CI on every change.

## Code Review Checklist

Before submitting a PR, verify:

- [ ] **Configuration safety**: Does the change preserve the last known-good configuration on failure?
- [ ] **Bounds checking**: Are inputs, queues, output, and subprocesses bounded?
- [ ] **Privacy**: Could any log include typed text, trigger names, or replacement contents?
- [ ] **Path safety**: Do path operations resolve symlinks and validate ownership?
- [ ] **Error classification**: Are errors marked as retryable vs. permanent?
- [ ] **Documentation**: Is the CLI/UI behavior documented with reproducible examples?
- [ ] **Systemd integration**: Are systemd changes least-privilege and compatible with user services?
- [ ] **Backend trait consistency**: Do backend implementations follow the trait contract?

## Key Conventions

### Fail-closed semantics

**What this means:**
- An `app_filter` with no known window must NOT match
- A word-boundary trigger whose context was evicted must NOT match
- A backend that can't establish injection must REFUSE, not silently fail
- An untrusted config path must be REJECTED at startup

**Why:** Safety over convenience. Silent degradation is worse than explicit errors.

### State compatibility

**Documentation and `wayexpand doctor` must not claim a backend works without evidence.** See `SUPPORT_MATRIX.md` for the promotion policy:
- `Supported` = verified, tested, production-ready
- `Experimental` = implemented, partially tested
- `Not supported` = not implemented, documented as unavailable

### CLI exit codes

Exit codes come from error message text. The function `exit_code_for` in `crates/cli/src/main.rs` pattern-matches messages like "usage:", "configuration invalid:", etc.

**Important:** Changing error message text changes exit codes. This is a documented contract in `docs/COMPATIBILITY.md` with contract tests in the CLI module.

**If you change an error message:** Update the corresponding exit code test.

### Error messages

**Never leak snippet content.** Use `ConfigError::safe_summary()` for user-facing output. The script `scripts/test-doctor.sh` checks for leaks.

Example:
```rust
// ✗ Bad: leaks snippet content
eprintln!("Invalid config: trigger = \"{}\"", trigger);

// ✓ Good: safe summary
eprintln!("{}", ConfigError::safe_summary());
```

### Systemd units

**User-level services only.** Never require root. Examples:
```ini
[Unit]
Description=WayExpand
After=graphical-session-pre.target

[Service]
Type=simple
ExecStart=/usr/bin/wayexpand-daemon
Restart=on-failure
RestartSec=10

[Install]
WantedBy=graphical-session.target
```

## Building and Testing

### Standard development

```sh
cargo build --locked --workspace
cargo test --locked --workspace

# Test a specific crate or test
cargo test --locked -p wayexpand-core app_filter
```

### Release build

```sh
cargo build --locked --release --workspace
cargo clippy --locked --workspace --all-targets --release -- -D warnings
```

### Offline builds (vendored dependencies)

```sh
# Generate vendor/ and a local Cargo source replacement
mkdir -p .cargo
cargo vendor vendor/ > .cargo/config.toml

# Now builds work without network
cargo build --locked --offline --workspace
```

See `CLAUDE.md` for dependency management.

### Formatting and linting

```sh
# Check formatting
cargo fmt --all -- --check

# Auto-fix formatting
cargo fmt --all

# Run clippy with strict settings (same as CI)
cargo clippy --locked --workspace --all-targets -- -D warnings

# For release builds
cargo clippy --locked --workspace --all-targets --release -- -D warnings

# Shell scripts
shellcheck scripts/*.sh
```

## Release Process

**Pre-release checks:**
```sh
cargo test --locked --workspace
cargo test --locked --release --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release --workspace
git diff --check  # No trailing whitespace
```

**Version management:**
Versions must match across:
- `Cargo.toml` (all crates)
- `debian/changelog`
- `PKGBUILD`
- RPM spec file

Use `scripts/prepare-release.sh` to update all at once.

**After release:**
- Push tags to GitHub
- Create release notes
- Update `CHANGELOG.md`
- Announce in discussions

See `docs/RELEASING.md` for full details.

## Commit Message Guidelines

**Style:** Clear, descriptive, present tense

**Format:**
```
category: short summary

Longer explanation if needed. Mention issue numbers.
Use conventional commits when appropriate: feat:, fix:, docs:, test:, refactor:

Lines should be <80 characters for readability.
```

**Examples:**
```
fix(P0): correct safe_mode policy enforcement

The safe_mode parameter was not being respected in policy checks.
This ensures violations are logged in audit mode but only enforced
when safe_mode=true. Fixes #42.

feat: add doctor --fix for safe corrections

Adds a new --fix flag that suggests and applies user-level corrections
like group membership for evdev, without modifying system configuration.
```

## Contributing Backend Integrations

**Backend structure:**
1. Create a new crate under `crates/backend-*`
2. Implement one or both traits:
   - `InputSource` — for input capture
   - `TextInjector` — for text output
   - `WindowTracker` — for window tracking (optional)
3. Add tests for all supported scenarios
4. Document prerequisites and limitations
5. Update `SUPPORT_MATRIX.md` with initial status

**Example trait implementation:**
```rust
use wayexpand_core::InputSource;

pub struct MyBackend {
    // State
}

impl InputSource for MyBackend {
    fn capture(&mut self) -> io::Result<InputEvent> {
        // Implement capture logic
    }
}
```

**Testing backends:**
- Retry logic with protocol failures
- Timeout handling
- Permission errors
- Malformed input/output
- Compositor reconnection

## Package Management

**Cargo.lock is committed** and authoritative. CI and installers use it to build reproducibly.

**Adding or updating dependencies:**
1. Update `Cargo.toml`
2. Run `cargo build --locked` to update `Cargo.lock`
3. Commit `Cargo.lock` with your changes
4. For offline builds, run `mkdir -p .cargo && cargo vendor vendor/ > .cargo/config.toml` and keep the generated `vendor/` directory with the build artifact

**Dependency policy:**
- Prefer stable, well-maintained crates
- Minimize dependencies in the core engine
- Backend crates can use compositor-specific libraries
- Document why a dependency is needed

## Continuous Integration

CI runs on every PR:
- **Formatting:** `cargo fmt --all -- --check`
- **Tests:** `cargo test --locked --workspace`
- **Linting:** `cargo clippy --locked --workspace --all-targets -- -D warnings` (both debug and release)
- **Shell scripts:** `shellcheck scripts/*.sh`
- **Integration tests:** `scripts/smoke-daemon.sh`, `scripts/test-doctor.sh`, etc.
- **Installer tests:** Verify installers work on clean systems
- **Systemd validation:** `systemd-analyze verify` on all units

**All checks must pass before merging.**

## Resources

- **Architecture details:** See `CLAUDE.md` in the repository
- **API contracts:** [docs/COMPATIBILITY.md](COMPATIBILITY.md)
- **Security model:** [SECURITY.md](../SECURITY.md)
- **Support matrix:** [docs/SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)
- **Changelog:** [CHANGELOG.md](../CHANGELOG.md)

## Getting Help

- **Documentation:** `docs/` directory
- **Issues:** https://github.com/itchyitchy123/wayexpand/issues
- **Questions and bug reports:** https://github.com/itchyitchy123/wayexpand/issues
- **Review process:** PRs are reviewed for correctness, safety, and adherence to conventions
