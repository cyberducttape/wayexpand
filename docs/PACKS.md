# WayExpand local packs

## Optional sysadmin and developer libraries

The seven administration and support packs are versioned and carry a source
review record in `packs/`; existing developer and operations-triage examples
remain available alongside them. Every pack is optional and works entirely
offline. Install only the libraries you want.

| Pack | Contents |
| --- | --- |
| `linux-administration` | Bash, SSH, systemd, and journalctl templates |
| `docker` | Docker, Podman, and Compose inspection templates |
| `kubernetes` | Parameterized, read-only `kubectl` inspection commands |
| `ansible` | Playbook, inventory, and role starters with syntax checks |
| `nginx` | Configuration checks, request probes, and server-block starters |
| `git` | Review-first Git commands and collaborative workflow templates |
| `customer-support` | Status updates, maintenance notices, and support replies |
| `developer` | SQL, shell, and Markdown starters |
| `ops-triage` | Read-only first-response checks (TLS expiry, disk, memory, sockets, DNS, HTTP, nginx, firewall, failed units) and on-call timeline and handoff notes |

Install and inspect a pack locally:

```bash
wayexpand pack inspect ./packs/kubernetes
wayexpand pack install ./packs/kubernetes
wayexpand pack list
wayexpand pack path wayexpand.kubernetes-operations
wayexpand validate --fleet
wayexpand reload
```

## Search a shared catalog

A catalog can be a checked-out Git repository or any local directory containing
pack directories (or a `packs/` subdirectory). Search works offline and reports
the pack version, publisher, review record, declared capabilities, command and
hotkey counts, and signature status against the configured signer list. It
does not install or activate results:

```bash
git clone https://example.org/team/wayexpand-packs.git ~/src/wayexpand-packs
wayexpand pack search ~/src/wayexpand-packs kubernetes
wayexpand pack search ~/src/wayexpand-packs --json
wayexpand pack inspect ~/src/wayexpand-packs/packs/kubernetes
wayexpand pack diff wayexpand.kubernetes-operations ~/src/wayexpand-packs/packs/kubernetes
wayexpand pack install ~/src/wayexpand-packs/packs/kubernetes --replace
```

Catalog search skips invalid pack directories and refuses a symlink as the
catalog root. It is a discovery aid, not a trust decision: read the diff and
permission disclosures, then install explicitly. Installation still checks
signature requirements, organization policy, version ordering, and command or
hotkey consent. The current installed manifest records the selected semantic
version; automatic updates and a registry-wide version lockfile are not
implemented yet.

Install copies a validated pack into
`${XDG_DATA_HOME:-~/.local/share}/wayexpand/packs/<id>`. No network or account
is used. Replacing an installed version requires `--replace`; lower versions
require the additional `--allow-downgrade` flag. The prior version is kept in
the sibling `wayexpand-pack-backups/` directory. `wayexpand pack remove <id>`
removes the active local copy. Admin policy still controls whether an
installed pack is eligible for fleet configuration.

Installed packs are ordinary editable TOML directories. To customize one,
locate it with `wayexpand pack path <id>`, edit files under `snippets/`, then
run `wayexpand validate --fleet` and `wayexpand reload`. For a durable
modification, update the source pack and its semantic version before sharing
or reinstalling it. Local changes invalidate any pack signature.

CI checks every pack in this repository with
`scripts/check-snippet-repo.sh --deny-risks packs`; run the same check on your
own snippet repository (see [SNIPPET_REPOSITORIES.md](SNIPPET_REPOSITORIES.md)).

`pack inspect` shows the version, review record, signature state, capabilities,
and action counts. The review record is descriptive repository metadata, not a
signature; a trusted signature authenticates the exact reviewed bytes. The
repository packs are unsigned. Managed deployments that require signed packs
must review and sign the selected content with their own trusted key.

### Review and install in the GUI

Open **Review local packs…** from the GUI menu and select a checked-out pack
directory. The review displays the publisher and review record, signature
verification state, declared and detected capabilities, exact replacement
text, command arguments and environment forwarding, hotkeys, and an update
diff when that pack is already installed. Commands are described for review;
the review itself does not execute them. Installing any pack that contains a
command or hotkey requires a separate explicit consent checkbox, and current
organization policy and signature requirements still apply.

The GUI binds installation to a SHA-256 fingerprint of the manifest and
snippet files shown during review. The CLI checks that fingerprint before
installation and verifies the staged copy again, so changing the source after
review causes installation to fail and asks for a new review. Review text is
displayed as untrusted content. If the GUI cannot display all of a pack or its
diff within its review limits, installation is disabled; use `wayexpand pack
inspect` in a terminal to examine it.

`pack import` still produces ordinary TOML on stdout and does not install or
activate snippets. It strips commands and hotkeys. `pack install` keeps the
validated pack structure intact; it refuses command and hotkey actions unless
you explicitly pass `--allow-actions` after reviewing them.

The snippets insert command text for the user to inspect. They do not run the
commands. Defaults favor inspection, explicit namespaces/targets, and bounded
queries where the command supports them. As with any template, review field
values before using the resulting command.

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

[review]
reviewed_by = "Operations maintainers"
reviewed_at = "2026-10-27"
summary = "Insert-only incident templates and read-only diagnostics"
```

Each file under `snippets/` is a normal WayExpand TOML configuration. Its
expansions are merged in lexical filename order. The pack manifest and every
snippet file are strict: unknown fields and invalid configuration are errors.
Pack `version` must be semantic versioning, and `id` is restricted to safe
directory-name characters. Review metadata must contain a reviewer, a real
calendar date, and a summary.

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

## Reviewing updates and rollback

Install keeps the previous validated pack in the private local backup
directory. Before replacing a pack, compare the candidate against the current
version:

```bash
wayexpand pack diff incident-response ./incident-response-2.4.0
wayexpand pack install ./incident-response-2.4.0 --replace --signers ./allowed_signers
```

The diff reports manifest changes, added/removed/changed snippet lines, and the
candidate's command, hotkey, and capability disclosures. Treat snippet text as
review-sensitive: the explicit diff command prints it to the terminal. Install
continues to enforce signature trust, version ordering, and command/hotkey
consent. To restore the most recently backed-up version:

```bash
wayexpand pack rollback incident-response --signers ./allowed_signers
```

Rollback revalidates the backup, enforces current organization policy and
required signatures, stages the restored pack, and preserves the currently
installed version as the next backup. It then asks the daemon to reload.

## Managed packs (organization policy)

In safe mode, `require_signed_packs = true` accepts only packs signed by a
signer in `pack_signers_file` (default `/etc/wayexpand/pack-signers`), which
must be root-owned and not writable by others; `pack import --signers FILE`
is refused rather than trusting a signers file the user chose. This applies to `pack import`
and to the fleet pack layer: a pack directory under
`${XDG_DATA_HOME:-~/.local/share}/wayexpand/packs/` that contains a manifest is loaded from its
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
are still removed by `pack import`. There is no central update channel or GUI
trust review; review candidate diffs and retain the signed source pack when
recording what was approved.

Pack loading treats pack contents as hostile input. The manifest is limited to
64 KiB; each snippet file to 1 MiB; the snippets directory to 1,024 entries;
and the aggregate snippet data to 16 MiB. Pack and snippet paths must be
regular files/directories (symlinks and special files are rejected), and the
pack is read and parsed once per operation.
