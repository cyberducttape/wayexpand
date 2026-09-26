# CLAUDE.md

> Developer-only guidance for Claude Code. This file is not product
> documentation and is intentionally excluded from the public documentation
> hierarchy.

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

System deps: `libwayland-dev libxkbcommon-dev pkg-config` (Debian/Ubuntu).

**Offline builds (optional):**
For offline builds without network access, generate vendored dependencies locally:
```sh
cargo vendor vendor/ > .cargo/config.toml
```
This creates a `vendor/` directory (ignored by git). With the generated source
replacement config, builds work without network access. A normal clone uses
crates.io and does not require `vendor/`.

**Standard development:**
```sh
cargo build --locked --workspace
cargo test --locked --workspace
cargo test --locked -p wayexpand-core app_filter      # tests whose name contains "app_filter"
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings   # CI also runs this with --release
```

Cargo.lock is committed and authoritative. When adding or bumping dependencies, Cargo.lock
is updated automatically. For offline builds, re-run `cargo vendor vendor/ > .cargo/config.toml` locally.

Clippy results can be stale from the build cache; `touch` a changed crate's `src/lib.rs`/`main.rs` before trusting a clean run.

Package names differ from directory names: `crates/cli` is package `wayexpand` (binary `wayexpand`); the others are `wayexpand-core`, `wayexpand-daemon`, `wayexpand-ui`, `wayexpand-gui`, and `wayexpand-backend-*`.

Script-level checks run in CI (`.github/workflows/ci.yml`): `scripts/test-release.sh`, `smoke-daemon.sh`, `test-doctor.sh`, `test-ui.sh`, the `test-install-*.sh` scripts, `shellcheck scripts/*.sh`, and `systemd-analyze verify` on the units in `systemd/`.

## Architecture

- `wayexpand-core` is the engine and has no backend dependencies. `ExpansionEngine::process(InputEvent)` returns `ExpansionResult`s. Matching is a reversed char trie (`matcher.rs`) over a bounded rolling buffer; `propagate_case` works by inserting case variants as extra triggers. `backend.rs` defines the `InputSource`, `TextInjector` and `WindowTracker` traits and `discover_backends()`, which is an environment-only guess (env vars, `/dev` nodes). It never talks to the compositor.
- Backends are separate crates implementing those traits: input-method-v2 (capture and output), evdev (capture only; explicit `/dev/input` authorization, with active-seat uaccess by default and the `input` group as a legacy fallback), libei and wlroots virtual-keyboard (output only), and KWin scripting over D-Bus (window tracking, KDE only). The shared resolver makes a conservative automatic choice when possible; explicit `--source=` / `--backend=` overrides remain available, and `wayexpand setup` selects a policy-aware compatibility mode.
- `wayexpand-daemon` runs the event loop. The control socket (`control.rs`) runs on its own thread and talks to the loop through atomics; `reload.rs` watches the config and swaps it only after the new one validates.
- `wayexpand` (CLI), `wayexpand-ui` (crossterm TUI) and `wayexpand-gui` (egui) are front ends that use the core directly. The CLI's `doctor` also runs live protocol probes from the backend crates.

## Conventions that aren't obvious from the code

- **Fail closed.** An `app_filter` snippet with no known window, or a word-boundary trigger whose preceding context was evicted, must not match. When an `app_id` is present it is the only thing an `app_filter` matches against; the window title is a fallback only when there is no `app_id`.
- **State compatibility precisely.** Docs and `doctor` must not claim a backend works without evidence (see `docs/SUPPORT_MATRIX.md`, promotion policy).
- **CLI exit codes come from typed error categories.** `CliErrorKind` carried by `CliError` in `crates/cli/src/main.rs` assigns stable exit codes independently of human-facing error wording. Exit codes and JSON output are a documented contract in `docs/COMPATIBILITY.md`, with contract tests in the CLI's test module.
- **Errors must not leak snippet content.** Use `ConfigError::safe_summary()` in user-facing output; `scripts/test-doctor.sh` checks for leaks.
- Open work and known bugs are tracked in `PROFESSIONAL_ROADMAP.md`.

## Packaging

- The Ubuntu PPA is built by a Launchpad recipe from a vendored source upload;
  a clean Git checkout is not sufficient because Launchpad builders do not
  have reliable crates.io access. Pushing to `origin` alone does not update it.
- `debian/rules` overrides `dh_clean` with `-X.orig`, because vendored crates' `Cargo.toml.orig` files are checksummed and dh_clean would delete them.
- Versions must match across `Cargo.toml`, `debian/changelog`, `PKGBUILD` and the RPM spec (`scripts/prepare-release.sh`, `docs/RELEASING.md`).
