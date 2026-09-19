# Contributing

WayExpand favors small, reviewable changes with explicit safety properties.
Keep the core engine platform-independent and put compositor-specific behavior
behind backend traits.

## Local workflow

```sh
git checkout -b feature/short-description
cargo fmt --all
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
shellcheck scripts/*.sh
```

Exercise behavior-level checks before opening a pull request:

```sh
bash scripts/smoke-daemon.sh
bash scripts/test-doctor.sh
bash scripts/test-ui.sh
bash scripts/test-install-user.sh
```

## Testing expectations

New parsing or matching behavior needs unit tests. Changes to lifecycle,
reload, permissions, or socket behavior need regression coverage at the
daemon/core boundary. Do not add tests that print real trigger or replacement
contents to logs.

For backend changes, document compositor prerequisites and test retry,
timeout, malformed protocol, and oversized-input behavior. A backend must fail
closed when it cannot establish a safe injection contract.

## Code review checklist

- Does the change preserve the last known-good configuration on failure?
- Are inputs, queues, output, and subprocesses bounded?
- Could any log include typed text, trigger names, or replacement contents?
- Does a path operation resolve symlinks and validate ownership before use?
- Are errors classified as retryable versus permanent?
- Is the CLI/UI behavior documented with a reproducible example?
- Are systemd changes least-privilege and compatible with user services?

## Release checklist

```sh
cargo test --locked --workspace
cargo test --locked --release --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release --workspace
git diff --check
```

Update the user-facing wiki when commands, defaults, security behavior, or
service semantics change. Keep `Cargo.lock` committed so CI and the installer
build the same dependency graph.
