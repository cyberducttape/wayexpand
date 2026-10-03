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

## Manifest capabilities and versions

A manifest can declare what its snippets need, and the oldest WayExpand that
understands it:

```toml
min_wayexpand_version = "1.3.0"
capabilities = ["broker_actions", "forms"]
allowed_actions = ["ticket-lookup"]
```

`capabilities` lists any of `commands`, `broker_actions`, `clipboard`, `env`,
and `forms`. When present, a pack that uses anything it does not declare is
rejected, as is a broker action missing from `allowed_actions`. A pack whose
`min_wayexpand_version` is newer than the running WayExpand is rejected.
`wayexpand pack inspect` shows the capabilities a pack actually uses.

## Signing packs

Packs are signed with ordinary OpenSSH keys, so an organization can reuse the
keys and `allowed_signers` files it already manages:

```bash
wayexpand pack sign ./support-team --key ~/.ssh/support_team_ed25519
wayexpand pack verify ./support-team --signers ./allowed_signers
```

The signature (`wayexpand-pack.sig`) covers a SHA-256 digest of the manifest
and every snippet file, so changing any of them invalidates it. Signers are
listed in OpenSSH `allowed_signers` format with the `wayexpand-pack`
namespace:

```text
support@example.com namespaces="wayexpand-pack" ssh-ed25519 AAAA...
```

`pack import` verifies a signature when one is present.

## Managed packs (organization policy)

In safe mode, `require_signed_packs = true` accepts only packs signed by a
signer in `pack_signers_file` (default `/etc/wayexpand/pack-signers`), which
must be root-owned and not writable by others. This applies to `pack import`
and to the fleet pack layer: a pack directory under
`~/.local/share/wayexpand/packs/` that contains a manifest is loaded from its
`snippets/` directory as managed, read-only snippets, separate from the user's
own library, and is excluded (and reported by `wayexpand fleet status`) unless
it verifies. Packs without a manifest count as unsigned. In audit mode the same
problems are reported but not enforced.

The workflow for an organization:

1. publish the pack and sign it with the team key;
2. install `/etc/wayexpand/pack-signers` and a safe-mode policy with
   `require_signed_packs = true`;
3. place the pack in the users' pack layer; WayExpand verifies the signature
   and the declared capabilities against policy before any snippet is used.

## Trust model and limits

The `publisher` field is descriptive; the signature is the identity. Commands
are still removed by `pack import`. There is no update channel or GUI trust
review yet; keep the original signed pack when recording what was approved.

Pack loading treats pack contents as hostile input. The manifest is limited to
64 KiB; each snippet file to 1 MiB; the snippets directory to 1,024 entries;
and the aggregate snippet data to 16 MiB. Pack and snippet paths must be
regular files/directories (symlinks and special files are rejected), and the
pack is read and parsed once per operation.
