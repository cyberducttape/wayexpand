# Operations

## Service lifecycle

After installation:

```sh
systemctl --user daemon-reload
systemctl --user enable --now wayexpand-input-method.service
systemctl --user status wayexpand-input-method.service
journalctl --user -u wayexpand-input-method.service -f
```

The input-method unit is the global capture path. The plain unit starts the
stdin harness and is useful for lifecycle and integration testing. Do not run
both: they share one configuration and control socket.

## Health contract

```sh
wayexpand status
wayexpand status --json
wayexpand doctor
wayexpand backend
```

Important status values:

- `config_state=ok`: the active configuration is known-good.
- `config_state=reload-rejected`: the last edit failed validation; the prior
  engine remains active.
- `state=reconnecting`: input or output transport is unavailable; matching is
  paused until a safe connection returns.
- `paused=true`: an operator explicitly disabled matching.

## Logging

The daemon uses `tracing` and supports normal `RUST_LOG` filters:

```sh
RUST_LOG=wayexpand_daemon=info wayexpand-daemon
RUST_LOG=wayexpand_daemon=debug systemctl --user restart wayexpand-input-method.service
```

Logs intentionally contain counts and sanitized failure categories, not typed
text, triggers, replacement bodies, or parser payloads.

## Upgrade and rollback

The installer does not overwrite configuration. To upgrade from a checkout:

```sh
git pull --ff-only
cargo test --locked --workspace
./scripts/install-user.sh
systemctl --user daemon-reload
systemctl --user restart wayexpand-input-method.service
wayexpand doctor
```

The previous binary is replaced during installation, while the TOML file is
preserved. Keep a copy of the configuration before major changes if you need
an operator-controlled rollback:

```sh
cp --preserve=mode ~/.config/wayexpand/expansions.toml /tmp/wayexpand-backup.toml
```

Restore through a private file and validate it before restarting:

```sh
install -m 600 /tmp/wayexpand-backup.toml ~/.config/wayexpand/expansions.toml
wayexpand validate
systemctl --user restart wayexpand-input-method.service
```

## Resource posture

The shipped units use an unprivileged user, private temporary/device/mount
namespaces, `NoNewPrivileges`, restrictive capabilities, bounded memory/tasks/
file descriptors, syscall architecture restrictions, and a controlled runtime
write path. These controls complement, rather than replace, file permission
validation.
