# WayExpand Wiki

WayExpand is a privacy-conscious, Wayland-first text-expansion daemon. It
turns short triggers into reusable text while keeping input capture, expansion
logic, and text insertion behind explicit, testable boundaries.

> Screenshot note: the images in this wiki are polished UI previews of the
> native editor and diagnostics experience. Exact colors and layout may vary
> slightly with the active desktop theme and eframe renderer.

## Start here

- [Getting started](Getting-Started.md) — install, validate, and run the first snippet.
- [GUI guide](GUI.md) — manage snippets, previews, imports, undo, and diagnostics.
- [Configuration reference](Configuration.md) — complete TOML schema and examples.
- [Operations](Operations.md) — systemd, health checks, logs, upgrades, and recovery.
- [Security model](Security.md) — trust boundaries, permissions, and fail-closed behavior.
- [Troubleshooting](Troubleshooting.md) — symptoms, causes, and safe fixes.
- [Contributing](Contributing.md) — development workflow, tests, release checklist, and style.

## Product map

```text
                    +----------------------+
                    |  wayexpand-gui / UI  |
                    |  edit + preview       |
                    +----------+-----------+
                               |
                         atomic TOML save
                               |
+-------------+       +-------v--------+       +----------------------+
| input-method| ----> | core engine    | ----> | wlroots / libei      |
| v2 source   |       | match + render |       | text insertion       |
+-------------+       +-------+--------+       +----------------------+
                               |
                         0600 control socket
                               |
                         wayexpand CLI
```

The default daemon mode is a deterministic stdin harness. Global desktop
capture is explicit with `--source=input-method`; output backends are explicit
with `--backend=wlroots` or `--backend=libei`.

Hotkey work begins with the shared core key-chord model. `Ctrl+Alt+M`,
`Super+Enter`, and common modifier aliases normalize to one representation so
future backends and the script runtime can dispatch consistently.

## Design promises

1. Invalid configuration never replaces the last known-good engine.
2. Typed text, trigger names, and replacements are not written to logs.
3. Configuration and control-socket paths are checked for trusted ownership
   and restrictive permissions before use.
4. Command expansions are direct child-process execution, never shell strings.
5. Input and output failures fail closed; ambiguous injections are not replayed.

## Quick path

```sh
cargo test --locked --workspace
cargo run --locked -p wayexpand-gui -- --help
./scripts/install-user.sh
wayexpand doctor
systemctl --user enable --now wayexpand-input-method.service
```

For a safe first run without compositor capture:

```sh
cargo run --locked -p wayexpand-daemon -- expansions.toml
printf '%s\n' ';;hello' | cargo run --locked -p wayexpand -- test ';;hello'
```
