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
produces the expected result, **run `wayexpand doctor`** to see which input and
output backends your compositor actually supports. Different compositors require
different backend combinations — there is no single "default" setup that works
everywhere.

## Requirements

- Linux with a Wayland session for global input capture.
- Rust stable and Cargo for building from source.
- `systemd --user` for the shipped service workflow.
- One of: input-method-v2 protocol support, libei/EIS, or access to `/dev/input`

The core engine and CLI can be built and tested without a compositor. Backend
and input source availability is environment-dependent; **always run
`wayexpand doctor` first** to see what your session actually supports before
starting any service.

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
`~/.local/bin`, installs three user units (wayexpand.service for testing,
wayexpand-input-method.service for compositors confirmed by `wayexpand doctor`,
wayexpand-evdev.service for raw-input capture), registers the desktop entry, and
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

First, check what your session actually supports:

```sh
wayexpand doctor
```

This shows you which input sources and output backends are available on your
compositor. For **KDE Plasma/KWin**, the most common setup is:

```sh
systemctl --user enable --now wayexpand-evdev.service
wayexpand status
```

For a compositor where `wayexpand doctor` confirms input-method-v2 support:

```sh
systemctl --user enable --now wayexpand-input-method.service
wayexpand status
```

⚠️ **Do NOT use wayexpand.service** — it's a test harness (stdin mode) and will
not provide global text expansion.

Enable exactly one service. Expected status includes `source=(stdin|input-method|evdev)`,
`backend=(none|input-method-v2|wlroots-virtual-keyboard|libei)`, `state=connected`,
and `config_state=ok`. A running service with `state=reconnecting` is intentionally
not injecting text until the compositor connection is safe again.

See [SUPPORT_MATRIX.md](../SUPPORT_MATRIX.md) for the complete backend compatibility matrix.

## Stop, pause, and inspect

```sh
wayexpand pause       # clear buffered input and disable matching
wayexpand resume      # re-enable matching
wayexpand status --json
wayexpand stop
```
