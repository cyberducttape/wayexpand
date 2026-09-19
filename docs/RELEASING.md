# Release procedure

WayExpand releases are created from annotated version tags. The release
workflow rejects a mismatch before publishing artifacts if the tag does not
match **both**:

- the workspace version in `Cargo.toml`
- the version in `debian/changelog`

The second check exists because Launchpad's PPA builds key off
`debian/changelog`, not the git tag or `Cargo.toml` — a tag that only
bumped `Cargo.toml` shipped a stale Launchpad build more than once before
this check was added. Bump both in the same commit you tag.

## Preparation

1. Run the complete verification suite from
   [`docs/wiki/Contributing.md`](wiki/Contributing.md).
2. Run the isolated release smoke test:

   ```sh
   bash scripts/test-release.sh
   ```

3. Review the support boundary in
   [`docs/SUPPORT_MATRIX.md`](SUPPORT_MATRIX.md).
4. Move completed `Unreleased` entries in `CHANGELOG.md` into a versioned
   section.
5. Update the workspace version in `Cargo.toml` **and** the top entry in
   `debian/changelog`, and regenerate `Cargo.lock` if required.
6. Commit the version and changelog update.

## Publish

```sh
git tag -a v<version> -m "WayExpand <version>"
git push origin v<version>
```

The release workflow runs the full CI verification suite (`ci.yml`) before
packaging, and publishing is skipped if it fails. It then builds the Linux
x86_64 binaries with the locked dependency graph and publishes:

**Binary archive** (for end users):
- `wayexpand-<version>-linux-x86_64.tar.gz` containing prebuilt binaries,
  systemd units, the desktop entry, application icon, example configuration,
  documentation, license, security policy, and installation scripts
- SHA256 checksum: `wayexpand-<version>-linux-x86_64.tar.gz.sha256`

**Source archives** (for distributions and offline builds):
- `wayexpand-<version>.tar.gz` (clean source, Cargo.lock only)
  - Recommended for AUR, Copr, and distributions that build from source
  - ~70 MB, cargo downloads dependencies from crates.io during build
  - Build systems add `cargo vendor vendor/` as needed
  - SHA256: `wayexpand-<version>.tar.gz.sha256`

- `wayexpand-<version>-vendored.tar.gz` (includes vendored dependencies)
  - For Launchpad PPA and offline/air-gapped builds
  - ~600 MB, all dependencies pre-downloaded
  - `CARGO_NET_OFFLINE=true` builds work without internet
  - SHA256: `wayexpand-<version>-vendored.tar.gz.sha256`

## Tarball distribution

**For package maintainers:**

| Target | Tarball | Build | Notes |
|--------|---------|-------|-------|
| AUR | `wayexpand-<version>.tar.gz` | `cargo build --release --locked` | Build system adds `cargo vendor vendor/` automatically |
| Copr (Fedora) | `wayexpand-<version>.tar.gz` | `cargo build --release --locked` | RPM spec includes `cargo vendor vendor/` in %build |
| Launchpad PPA | `wayexpand-<version>-vendored.tar.gz` | `dh build --buildsystem=cargo` | Debian rules handles `cargo vendor vendor/` |
| Source distribution | `wayexpand-<version>.tar.gz` | Any | Cleaner, more professional appearance (70 MB vs 600 MB) |
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

Install only from a verified release artifact. Keep the previous binary and
configuration backup available until the new service has passed `wayexpand
doctor --json` and a real expansion test.

```sh
tar -xzf wayexpand-<version>-linux-x86_64.tar.gz
cd wayexpand-<version>-linux-x86_64
./scripts/install-release.sh --enable
```

To remove a release-tarball or source-tree installation later, run
`scripts/uninstall-user.sh` (pass `--purge` to also delete the configuration
directory).
