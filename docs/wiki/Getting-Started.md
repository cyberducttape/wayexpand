# Getting started

WayExpand turns short triggers into reusable text. The safest first workflow
is to validate a configuration and simulate an expansion before enabling any
Wayland capture service.

```sh
wayexpand validate
wayexpand test ';;hello'
wayexpand test ';;hello' --json
```

The simulation is a dry run: it cannot type into another application. Once it
produces the expected result, use `wayexpand doctor` and follow the backend
support matrix before enabling a service.

## Requirements

- Linux with a Wayland session for global input capture.
- Rust stable and Cargo for building from source.
- `systemd --user` for the shipped service workflow.
- A compositor implementing input-method-v2 for the integrated global source.

The core engine and CLI can be built and tested without a compositor. Backend
availability is environment-dependent; `wayexpand doctor` reports what the
current session supports.

## Build and test

```sh
git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand
cargo test --locked --workspace
cargo build --locked --release --workspace
```

The CI-equivalent checks are:

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
shellcheck scripts/*.sh
bash scripts/smoke-daemon.sh
bash scripts/test-doctor.sh
bash scripts/test-ui.sh
bash scripts/test-install-user.sh
```

## Install for one user

```sh
./scripts/install-user.sh
export PATH="$HOME/.local/bin:$PATH"
systemctl --user daemon-reload
wayexpand doctor
```

The installer builds release binaries, installs them under
`~/.local/bin`, installs both user units, registers the desktop entry, and
creates `~/.config/wayexpand/expansions.toml` only when it does not exist.
Existing configuration is never overwritten.

## First snippet

Edit the configuration with the GUI:

```sh
wayexpand-gui
```

Or add this minimal file manually:

```toml
[[expansion]]
trigger = ";;hello"
replacement = "Hello from WayExpand!"
description = "A first test snippet"
tags = ["demo"]
```

Validate before starting the daemon:

```sh
wayexpand validate
wayexpand preview ';;hello'
wayexpand test ';;hello'
```

## Start the real source

Enable exactly one service:

```sh
systemctl --user enable --now wayexpand-input-method.service
systemctl --user status wayexpand-input-method.service
wayexpand status
```

Expected status includes `source=input-method`, `backend=input-method-v2`,
`state=connected`, and `config_state=ok`. A running service with
`state=reconnecting` is intentionally not injecting text until the compositor
connection is safe again.

## Stop, pause, and inspect

```sh
wayexpand pause       # clear buffered input and disable matching
wayexpand resume      # re-enable matching
wayexpand status --json
wayexpand stop
```
