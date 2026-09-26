# Release procedure

WayExpand releases are created from annotated `vX.Y.Z` version tags. The
unprefixed form (`X.Y.Z`) is not a release reference and must not be reused.
The release
workflow rejects a mismatch before publishing artifacts if the tag does not
match all package metadata:

- the workspace version in `Cargo.toml`
- the version in `debian/changelog`
- `PKGBUILD`
- `wayexpand.spec`
- the AppStream release metadata
- the IBus component metadata

The CLI also embeds the commit identity. Clean builds from an exact `v<version>`
tag report the release version; other Git builds report the package version with
`-dev+<short-sha>`. Source archives without Git metadata retain the package
version and report an `unknown` commit, so release evidence should always retain
the generated artifact metadata.

The second check exists because Launchpad's PPA builds key off
`debian/changelog`, not the git tag or `Cargo.toml` — a tag that only
bumped `Cargo.toml` shipped a stale Launchpad build more than once before
this check was added. Bump both in the same commit you tag.

The release workflow uses the pinned Rust 1.96.0 toolchain and separately
checks the declared Rust 1.87 MSRV in CI. A release must not depend on
whichever `stable` toolchain happens to be installed on the runner; update the
workflow pin deliberately when changing the release compiler.

## Preparation

1. Run the complete verification suite from
   [DEVELOPMENT.md](DEVELOPMENT.md) (see Release Process section).
   A stable release must not contain an ignored test documenting a known
   production bug. Any intentionally ignored test must name an explicitly
   accepted issue and release disposition; otherwise it is a release blocker.
2. Run the isolated release smoke test:

   ```sh
   bash scripts/test-release.sh
   ```

3. Review the support boundary in
   [`docs/SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md).
4. Move completed `Unreleased` entries in `CHANGELOG.md` into a versioned
   section.
5. Run `./scripts/prepare-release.sh <version>`. It updates the workspace,
   changelogs, distro metadata, AppStream, and the IBus component, computes the
   source archive checksum from the staged release tree, and creates the
   complete maintainer-authored commit and tag. Review the generated release
   section before publishing; no post-tag metadata edits are expected.
6. Regenerate `Cargo.lock` if dependency versions changed, then run the
   release checks again.

## Publish

```sh
git tag -a v<version> -m "WayExpand <version>"
git push origin v<version>
```

The release workflow runs the full CI verification suite (`ci.yml`) before
packaging, and publishing is skipped if it fails. It then builds the Linux
x86_64 binaries with the locked dependency graph and publishes:

To republish artifacts for an existing tag, use the workflow's manual
dispatch and enter that tag in the `release_ref` field (for example,
`v1.2.0`). This is the recovery path when a GitHub release exists but its
artifacts were not uploaded; it checks out and verifies the selected tag and
produces the same vendored source archive required by Launchpad.

Release archives must be created by the release workflow or with
`git archive`/`scripts/generate-release-tarballs.sh`. Do not run `tar` on a
working checkout: that can include `.git/` metadata and build outputs such as
`target/`. The release workflow verifies that published archives contain no
`.git/` directory.

**Binary archive** (for end users):
- `wayexpand-<version>-linux-x86_64.tar.gz` containing prebuilt binaries,
  systemd units, the desktop entry, application icon, example configuration,
  current operational documentation, license, security policy, and installation
  scripts. Historical material under `docs/archive/` remains in Git but is not
  shipped in this end-user archive.
- SHA256 checksum: `wayexpand-<version>-linux-x86_64.tar.gz.sha256`
- Cargo dependency inventory: `wayexpand-<version>-linux-x86_64.cargo-metadata.json`

**Source archives** (for distributions and offline builds):
- `wayexpand-<version>.tar.gz` (clean source with Cargo.lock; no vendored source config or `vendor/`)
  - Recommended for AUR, Copr, and distributions that build from source
  - Contains tracked project sources; Cargo downloads dependencies from crates.io during build
  - Build systems add `cargo vendor vendor/` as needed
  - SHA256: `wayexpand-<version>.tar.gz.sha256`

- `wayexpand-<version>-vendored.tar.gz` (includes vendored dependencies)
  - For Launchpad PPA and offline/air-gapped builds
  - Includes dependencies for offline builds; archive size varies with the locked dependency graph
  - `CARGO_NET_OFFLINE=true` builds work without internet
  - SHA256: `wayexpand-<version>-vendored.tar.gz.sha256`

The Launchpad recipe must build from the vendored source archive (or an
equivalent source upload containing both `vendor/` and `.cargo/config.toml`).
It must not build the clean Git checkout: Launchpad builders do not have
reliable crates.io access, and Debian packaging fails closed rather than
attempting a network dependency download.

The workflow pins every GitHub Action to a full commit SHA. Do not replace
those pins with moving version tags during release-workflow maintenance.

## Tarball distribution

**For package maintainers:**

| Target | Tarball | Build | Notes |
|--------|---------|-------|-------|
| AUR | `wayexpand-<version>.tar.gz` | `cargo build --release --locked` | Build system adds `cargo vendor vendor/` automatically |
| Copr (Fedora) | `wayexpand-<version>.tar.gz` | `cargo build --release --locked` | RPM spec includes `cargo vendor vendor/` in %build |
| Launchpad PPA | `wayexpand-<version>-vendored.tar.gz` | `dpkg-buildpackage -b` | Debian rules invoke the declared Cargo toolchain directly and use the vendored archive offline |
| Source distribution | `wayexpand-<version>.tar.gz` | Any | Registry access required; no project source replacement config |
| Offline build | `wayexpand-<version>-vendored.tar.gz` | `CARGO_NET_OFFLINE=true` | All dependencies included, no network required |

Do not call a release stable while [`docs/SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md)
still marks key pass-through or a compositor's coverage as unsupported or
experimental. Release notes must name known backend limitations and any
configuration migration behavior.

## Verification after publishing

Download the archive and checksum independently, then verify and inspect
it:

```sh
sha256sum --check wayexpand-<version>-linux-x86_64.tar.gz.sha256
tar -tzf wayexpand-<version>-linux-x86_64.tar.gz
```

The `.sha256` files provide integrity checks, not publisher identity. Verify
the GitHub release attestation when one is published, and verify the release
tag is the signed/expected commit before installing. The Cargo metadata JSON
is an SBOM-style dependency inventory; compare it with the source archive and
retain it with your deployment record.

Install only from a verified release artifact. Keep the previous binary and
configuration backup available until the new service has passed `wayexpand
doctor --json` and a real expansion test.

```sh
tar -xzf wayexpand-<version>-linux-x86_64.tar.gz
cd wayexpand-<version>-linux-x86_64
./scripts/install-release.sh
wayexpand explain-backend
# Enable the selected service explicitly; do not implicitly enable
# input-method-v2 because unsupported non-text keys may be lost.
```

To remove a release-tarball or source-tree installation later, run
`scripts/uninstall-user.sh` (pass `--purge` to also delete the configuration
directory). If evdev access was enabled, this removes user files only; revoke
raw-input privileges separately with
`sudo scripts/install-evdev-permissions.sh --uninstall` or the distro helper
`sudo wayexpand-install-evdev-access --uninstall`.
