# Packaging WayExpand

This guide covers building and maintaining WayExpand packages for different Linux distributions.

## Rust toolchain policy

WayExpand supports Rust **1.95 or newer** for source builds. This is the
project MSRV and is declared in the workspace `Cargo.toml`; CI checks Rust 1.95
and the pinned project toolchain in `rust-toolchain.toml` (currently Rust
1.96.0). End users installing a release `.deb`, `.rpm`, PPA package, or binary
archive do not need Rust or Cargo on the target machine. Only distribution
packagers and source-build users need a compatible toolchain.

## Quick Reference

**Current publication status:** the v1.3.3 GitHub release has no downloadable
assets, and the Launchpad PPA currently publishes no WayExpand binaries. Treat
all package formats below as build/packaging paths, not as currently available
downloads. Release publication remains gated on compositor certification.

| Distro | Package | Status | Maintainer |
|--------|---------|--------|------------|
| Arch Linux | `wayexpand` | Build locally | Packaging preview; not submitted to AUR |
| Ubuntu | `wayexpand` | No published PPA binaries | Source packaging path exists; signed upload and successful series build are pending |
| Debian | `.deb` | Build locally from source | No release asset or APT repository currently published |
| Fedora/RHEL | `.rpm` | Build locally from source/spec | No release asset or Copr repository currently published |
| aarch64 | `wayexpand` | Build locally from source | No release archive currently published |

## Building Locally

### Arch Linux (packaging preview)

```bash
# Clone and build
git clone https://github.com/cyberducttape/wayexpand.git
cd wayexpand
makepkg -si
```

**To maintain:**
1. Follow [RELEASING.md](RELEASING.md) to update version numbers across all files
2. Update `pkgver` and `pkgrel` in `PKGBUILD` (happens automatically with `prepare-release.sh`)
3. SHA256 checksum is computed and inserted during release workflow
4. Test with `makepkg -si`
5. Before publication, generate `.SRCINFO` and verify in a clean Arch chroot

### Ubuntu PPA / Debian Vendored Build

```bash
# Debian packaging expects the vendored release archive for offline builds.
# Do not build the clean Git checkout: Launchpad builders cannot fetch crates.io.
# Replace ${VERSION} with the current release version (e.g., 1.2.0)
tar -xzf wayexpand-${VERSION}-vendored.tar.gz
cd wayexpand-${VERSION}

# Build source package
dpkg-buildpackage -us -uc

# Or build binary package
dpkg-buildpackage -b

# Install locally
sudo dpkg -i ../wayexpand_${VERSION}-1_amd64.deb
```

This documents a local packaging path; it does not imply that a package is
currently published. Check the actual GitHub release and PPA before installing
or deploying. The release workflow only builds artifacts after its certification
gates pass.

**To maintain:**
1. Update version in `debian/changelog`
2. Run `dch -i` to manage changelog entries
3. Test build: `debuild -us -uc`
4. Push to Launchpad PPA

### Fedora/RHEL

**Status:** New GitHub releases are configured to include a local-install RPM
for x86_64. The RPM spec is maintained and built in CI, but no official Copr
repository is published, so DNF repository-based updates are not yet available.

For the release RPM, install the downloaded local package with:

```bash
sudo dnf install ./wayexpand-<version>-1.x86_64.rpm
```

**Local build from spec file:**

```bash
# Prepare for rpmbuild
rpmbuild -ba wayexpand.spec

# Or use mock for clean builds
# Replace ${VERSION} with the current release version (e.g., 1.2.0)
mock wayexpand-${VERSION}-1.fc39.src.rpm
```

**To build from the repository spec file:**
```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
rpmbuild -ba wayexpand.spec
```

**To set up an official Copr repository:**

1. Create account at https://copr.fedorainfracloud.org
2. Create new project
3. Configure to auto-build from GitHub releases
4. Announce in README

**Contributions welcome:** If you maintain a Copr repo or want to create one, please open an issue or PR.

### aarch64

The release workflow builds on native x86_64 and aarch64 runners and attaches
both archives to new tagged releases. Older releases may have only x86_64.
On an aarch64 Fedora, Debian, Ubuntu, or Arch system, either use the matching
release archive or build from source after installing Rust 1.95+ and native
Wayland dependencies:

```bash
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev
cargo build --locked --release --workspace
```

The binaries are in `target/release/`. Run `wayexpand doctor` before enabling
a user service. Native builds are preferred over cross-builds because the
Wayland and compositor protocol libraries must match the target system.

CI uses native aarch64 hardware, avoiding fragile cross-builds of the Wayland
and compositor-protocol stack. The release archive uses the same user installer
and assets as x86_64.

---

## Submission Instructions

### AUR (Arch Linux User Repository)

**First Time:**
1. Create AUR account at https://aur.archlinux.org
2. Add SSH public key to account
3. Clone empty repo: `git clone ssh://aur@aur.archlinux.org/wayexpand.git`
4. Copy `PKGBUILD` and `.gitignore` to repo
5. Generate `.SRCINFO`: `makepkg --printsrcinfo > .SRCINFO`
6. Commit and push

**Updates:**
```bash
cd wayexpand-aur
# Update PKGBUILD with new version (see RELEASING.md for version update process)
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "Update to v${VERSION}"  # Replace ${VERSION} with current version
git push
```

### Ubuntu PPA

To create an unsigned Debian source package for Launchpad, manually dispatch
the GitHub Release workflow with `launchpad_series` set to the Ubuntu series
codename. The workflow targets that suite in `debian/changelog`; it intentionally
does not guess a PPA target for tag-push releases. The source package is built
from the vendored archive; the clean Git recipe does not contain `vendor/` and
must not be used for offline PPA builds. Download the `.dsc`, `.orig.tar.gz`,
`.debian.tar.xz`, `_source.changes`, and `_source.buildinfo` files into one
directory, then sign and upload the changes file with the Launchpad-upload GPG
key:

```bash
source_version=1.3.3-2 # use the version shown in the .dsc filename
debsign "wayexpand_${source_version}_source.changes"
dput ppa:cyberducttape/ppa "wayexpand_${source_version}_source.changes"
```

Launchpad builds from source, so only submit to Ubuntu series whose build
archive provides both `rustc` and `cargo` at or above the project MSRV (1.95).
Ubuntu 26.04 Resolute currently provides Rust 1.93 and cannot satisfy this
package's build dependencies. The configured `main-daily` recipe also tracks
clean `main` rather than the vendored release source; it is not a supported
automatic PPA publishing path. Check each Launchpad build result before
advertising a series as installable.

**Note:** See [RELEASING.md](RELEASING.md) for the authoritative release version workflow
(it is the single source of truth for version numbers across all distributions).

### Fedora/Copr

**First Time:**
1. Create account at https://copr.fedorainfracloud.org
2. Create new project
3. Upload spec file and source tarball

**Updates:**
1. Update spec file
2. Re-upload or let Copr auto-rebuild from GitHub releases

---

## Testing Installations

### Test Arch packaging
```bash
# In a clean chroot
archiso-mount-rw
makepkg -si
wayexpand-gui
```

### Test Debian
```bash
# In a container
docker run -it debian:bookworm bash
# Build/install from the vendored source archive; no published package is
# currently available for an installation test.
wayexpand-gui
```

### Test Fedora
```bash
# In a container
docker run -it fedora:39 bash
# No official Copr exists yet; build locally from the spec/source package.
rpmbuild -ba wayexpand.spec
wayexpand-gui
```

---

## Versioning and Release Flow

**⚠️ IMPORTANT:** This document is for packaging maintainers. The authoritative
release workflow is documented in [RELEASING.md](RELEASING.md).

Do not edit version numbers independently in `PKGBUILD`, `debian/changelog`, or
`wayexpand.spec`. All version updates must go through the central release
process defined in RELEASING.md, which ensures consistency across:
- `Cargo.toml`
- `debian/changelog`
- `PKGBUILD`
- `wayexpand.spec`
- Release metadata (AppStream metainfo, IBus component XML)

When a new release is tagged, the version numbers in all packaging files are
already updated. Your packaging job is to:

1. **Build locally and test** on each target distro
2. **Submit/upload to each distro** using the updated version numbers
3. **Report successful distribution** back to the project

See [RELEASING.md](RELEASING.md) section "Release Checklist" for the complete workflow.

---

## Automated Updates

Potential future work, not active publication paths:
- **Copr webhook** after an official Copr repository is created
- **Ubuntu PPA automation** after a supported series is selected and successful builds are published

GitHub release workflows are configured, but release artifacts remain gated
on certification. Do not treat workflow configuration as evidence of published
packages.

---

## Vendored Dependencies

The working repository does not need to keep `vendor/` checked in. Normal
online builds use `Cargo.lock` and fetch crates from the configured Cargo
registry.

Release tooling supports two source archive shapes:

- `wayexpand-<version>.tar.gz`: clean source archive without `vendor/`
- `wayexpand-<version>-vendored.tar.gz`: offline-build archive with `vendor/`
  and the generated Cargo source replacement config

Use the clean archive for build systems that can access Cargo registries and
populate Cargo's cache before an offline/frozen build (the Arch PKGBUILD does
this in `prepare()`). Use the vendored archive for Launchpad, Fedora RPM builds,
air-gapped builders, or any policy requiring all Rust dependencies to be in the
source upload. Large local `vendor/` directories are build artifacts, not
required repository content.

---

## References

- [ArchWiki: Creating packages](https://wiki.archlinux.org/title/Creating_packages)
- [Debian New Maintainers' Guide](https://www.debian.org/doc/manuals/maint-guide/)
- [Fedora Package Maintenance Guide](https://docs.fedoraproject.org/en-US/package-maintainers/)
