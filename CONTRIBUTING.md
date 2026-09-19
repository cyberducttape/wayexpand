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

- Rust 1.93+ (install via [rustup](https://rustup.rs/))
- Standard tools: `git`, `make`, `pkg-config`
- Wayland dev libraries (usually pre-installed on Wayland systems)

### Setup and Testing

```bash
git clone https://github.com/itchyitchy123/wayexpand
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

3. **Write a clear commit message**:
   ```
   [type]: Brief description (present tense)
   
   Longer explanation of the change and why it's needed.
   
   Types: feat, fix, docs, test, refactor, perf, chore
   ```

4. **Reference issues** — Link related issues in your PR description

## Development Guide

Detailed development policy and workflows are in [`docs/wiki/Contributing.md`](docs/wiki/Contributing.md).

### Project Structure

- **`crates/core`** — Expansion engine (platform-independent)
- **`crates/daemon`** — Service coordinating input/output backends
- **`crates/cli`** — Command-line interface
- **`crates/gui`** — Native egui-based GUI editor  
- **`crates/backend-*`** — Wayland protocol implementations
- **`vendor/`** — Generated, ignored dependency source for reproducible
  offline Debian/Launchpad builds. Normal development resolves from crates.io;
  see the offline-build instructions in `docs/DEVELOPMENT.md`.

### Common Tasks

**Add a new CLI command:** Add logic to `crates/cli/src/main.rs` and tests to `crates/core/src/engine.rs`

**Fix a GUI bug:** Edit `crates/gui/src/main.rs`, test with `cargo run -p wayexpand-gui`

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

- Open a [GitHub Discussion](https://github.com/itchyitchy123/wayexpand/discussions) for questions
- Check [`docs/wiki/`](docs/wiki/) for detailed documentation
- Review [`SECURITY.md`](SECURITY.md) for security concerns

Thank you for contributing to WayExpand!
