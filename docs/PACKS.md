# WayExpand local packs

WayExpand supports a deliberately small local pack format. Packs are
directories, not downloaded archives, in this first version:

```text
incident-pack/
├── wayexpand-pack.toml
└── snippets/
    ├── operations.toml
    └── support.toml
```

`wayexpand-pack.toml` is metadata, not executable policy:

```toml
format_version = 1
id = "example-operations"
version = "1.0.0"
name = "Example Operations"
publisher = "Example Team"
description = "Common incident-response snippets"
```

Each file under `snippets/` is a normal WayExpand TOML configuration. Its
expansions are merged in lexical filename order. The pack manifest and every
snippet file are strict: unknown fields and invalid configuration are errors.

## Inspect and import

Inspect a pack before importing it:

```bash
wayexpand pack inspect ./incident-pack
```

Import writes a validated configuration to stdout:

```bash
wayexpand pack import ./incident-pack > /tmp/incident-pack.toml
wayexpand validate /tmp/incident-pack.toml
```

The importer reports command-backed entries and removes their commands from
the generated configuration. This is intentional: importing downloaded or
shared content must not silently grant process-execution privileges. Review
the pack, restore any commands manually, and validate the result only after
you have decided to trust each executable.

Hotkeys from packs are also disabled on import because their action is always
command-backed. Organization policy can prohibit commands globally with
`disable_commands = true`, require absolute command paths, and restrict pack
names through `allowed_packs`.

## Trust model and current limits

The current format is local and unsigned. The `publisher` field is descriptive
metadata, not a cryptographic identity. Do not treat it as proof of provenance.
Signature files, publisher keys, version pinning, updates, and a GUI trust
review are intentionally not implemented yet.

Until signed distribution exists:

- inspect pack contents before import;
- keep packs in a user-controlled directory;
- do not enable commands from an unreviewed pack;
- use organization policy for managed deployments;
- retain the original pack when recording what was approved.

This conservative boundary is preferable to presenting an unsigned public
marketplace as trusted.
