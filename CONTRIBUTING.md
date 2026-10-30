# Contributing to WayExpand

We welcome contributions! Whether you're fixing bugs, adding features, or improving documentation, your work helps make WayExpand better for everyone.

## Core Principles

Every change should preserve the project's operational guarantees:

- invalid configuration never replaces a working configuration
- user text and snippet contents never enter logs
- input, output, queues, and child processes remain bounded
- compositor-specific behavior stays behind backend boundaries
- known support limitations are documented with reproducible evidence

## Quick Start

### Prerequisites

- Rust 1.92+ (install via [rustup](https://rustup.rs/)). 1.92 is the minimum
  supported version (`rust-version` in `Cargo.toml`, checked by CI's MSRV job);
  `rust-toolchain.toml` pins 1.96.0 for development and release builds, and
  rustup installs it automatically. Code must still build with 1.92.
- Standard tools: `git`, `pkg-config`, `shellcheck`
- Development headers: `libwayland-dev` and `libxkbcommon-dev` on
  Debian/Ubuntu (`wayland-devel` and `libxkbcommon-devel` on Fedora)

### Setup and Testing

```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand

# Build
cargo build --release

# Test locally
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
shellcheck scripts/*.sh

# Run
./target/release/wayexpand doctor       # Check system configuration
./target/release/wayexpand-gui          # Launch GUI
```

## Before Opening a Pull Request

1. **Run all checks** (same as CI):
   ```sh
   cargo fmt --all -- --check
   cargo test --locked --workspace
   cargo clippy --locked --workspace --all-targets -- -D warnings
   shellcheck scripts/*.sh
   git diff --check
   ```

2. **Update CHANGELOG.md** — Add entries under `[Unreleased]` if your PR adds features or fixes bugs

3. **Write a clear commit message**: an imperative subject line without a
   type prefix (for example, `Fail closed when evdev output monitor is
   absent`), then a body explaining what changed and why. CI also enforces
   the commit-identity policy described in
   [`docs/REPOSITORY_GOVERNANCE.md`](docs/REPOSITORY_GOVERNANCE.md); pull
   requests are merged with merge commits, not squash or rebase merges.

4. **Reference issues** — Link related issues in your PR description

5. **Keep support claims evidence-based** — If a change affects a backend,
   compositor, installer, or certification result, update the relevant support
   matrix and include the exact environment and command used to obtain the
   evidence. A passing unit test does not establish compositor certification.

## Development Guide

Detailed development policy and workflows are in [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

### Project Structure

Package names differ from directory names: `crates/cli` is the `wayexpand`
package; the others are `wayexpand-<directory>`, except `action-broker`.

- **`crates/core`** — Expansion engine, configuration, templates, and packs
  (no backend dependencies)
- **`crates/daemon`** — Event loop, control socket, reload, and output
  transactions
- **`crates/cli`** — `wayexpand` command-line interface; commands live in
  `src/commands/`
- **`crates/gui`** — Native egui editor (`wayexpand-gui`); screens live in
  `src/app/`
- **`crates/ui`** — Terminal settings app (`wayexpand-ui`)
- **`crates/backend-*`** — Capture, injection, and window-tracking backends;
  `backend-selection` chooses among them
- **`crates/action-broker`**, **`broker-protocol`**, **`broker-client`** —
  Separately sandboxed execution of named command actions
- **`crates/process-supervisor`**, **`build-info`**, **`e2e`** — Process-group
  lifecycle, version stamping, and headless KWin/Sway test helpers
- **`vendor/`** — Generated, ignored dependency source for reproducible
  offline Debian/Launchpad builds. Normal development resolves from crates.io;
  see the offline-build instructions in `docs/DEVELOPMENT.md`.

### Common Tasks

**Add a new CLI command:** Add the command under `crates/cli/src/commands/`,
dispatch it from `crates/cli/src/main.rs`, list it in `wayexpand help` and
`docs/wayexpand.1`, and add tests in `crates/cli/src/tests.rs`. Exit codes and
JSON output are a contract (`docs/COMPATIBILITY.md`).

**Fix a GUI bug:** Find the screen under `crates/gui/src/app/`, add a headless
test in `crates/gui/src/tests.rs`, and check the result with
`cargo run -p wayexpand-gui` or `scripts/gui-screenshot.sh`.

**Add a feature:** Write tests first, then implementation, then documentation

## Reporting Issues

**For bugs:** Include output from `wayexpand doctor --json` and minimal reproduction steps. Do not include personal expansion content.

**For security issues:** Follow [`SECURITY.md`](SECURITY.md) for responsible disclosure.

## Code Review

PRs are reviewed by maintainers for:
- Correctness and safety
- Adherence to project principles (bounded resources, no logging of user content, etc.)
- Test coverage
- Documentation updates
- Code style (enforced by `rustfmt` and `clippy`)

## Questions?

- Open a [GitHub issue](https://github.com/cyberducttape/wayexpand/issues) for questions
- Check [`docs/`](docs/) for detailed documentation
- Review [`SECURITY.md`](SECURITY.md) for security concerns

Thank you for contributing to WayExpand!
