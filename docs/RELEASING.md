# Release procedure

WayExpand releases are created from annotated version tags. The tag must
match the workspace version in `Cargo.toml`; the release workflow rejects a
mismatch before publishing artifacts.

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
5. Update the workspace version in `Cargo.toml` and regenerate `Cargo.lock` if
   required.
6. Commit the version and changelog update.

## Publish

```sh
git tag -a v0.1.0 -m "WayExpand 0.1.0"
git push origin v0.1.0
```

The release workflow builds the Linux x86_64 binaries with the locked
dependency graph and publishes a tarball containing binaries, systemd units,
the desktop entry, documentation, license, and security policy. A SHA256
checksum is published beside the archive.

Do not call a release stable while the support matrix still marks key
pass-through or compositor coverage as unsupported. Release notes must name
known backend limitations and any configuration migration behavior.

## Verification after publishing

Download the archive and checksum independently, then verify and inspect it:

```sh
sha256sum --check wayexpand-0.1.0-linux-x86_64.tar.gz.sha256
tar -tzf wayexpand-0.1.0-linux-x86_64.tar.gz
```

Install only from a verified release artifact. Keep the previous binary and
configuration backup available until the new service has passed `wayexpand
doctor --json` and a real expansion test.
