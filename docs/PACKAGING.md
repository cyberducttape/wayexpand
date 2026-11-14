# Packaging WayExpand

This guide covers building and maintaining WayExpand packages for different Linux distributions.

## Rust toolchain policy

WayExpand supports Rust **1.92 or newer** for source builds. This is the
project MSRV and is declared in the workspace `Cargo.toml`; CI checks Rust 1.92
and the pinned project toolchain in `rust-toolchain.toml` (currently Rust
1.96.0). End users installing a release `.deb`, `.rpm`, PPA package, or binary
archive do not need Rust or Cargo on the target machine. Only distribution
packagers and source-build users need a compatible toolchain.

## Quick Reference

<!-- generated-install-status:start -->
**Publication status from the shared manifest (audited 2026-11-11):** GitHub 1.3.3 binary assets published: False. Launchpad PPA: experimental daily builds for ['amd64'].

| Distribution | PPA package status |
|---|---|
| Ubuntu 26.04 (Resolute) | 1.3.3 — current |
| Ubuntu 26.10 (Stonking) | 1.3.3 — current |
| Ubuntu 24.04 (Noble) | not published — unavailable |
| Ubuntu 22.04 (Jammy) | not published — unavailable |

Fedora Copr published: False; Arch AUR submitted: False. Package CI targets: Ubuntu 24.04, Ubuntu 26.04, Fedora 45. Physical desktop certification: False. Previous-package upgrade CI passed: False.
<!-- generated-install-status:end -->

The release payload is built on Ubuntu 24.04. Package lifecycle CI is
configured to install a 1.3.2 package fixture, upgrade to the candidate, roll
back to 1.3.2, re-upgrade, reinstall, and remove it on the listed package
targets. The previous package is deliberately older than the current 1.3.3
candidate. This checks package-manager migration and files, not GNOME session
behavior; the configured test remains unverified until a complete CI run passes.
A GNOME session after package upgrade still requires the separate desktop
acceptance gate; package CI does not establish desktop input certification or a
stable distro repository. GitHub release artifacts receive
build-provenance attestations and checksums; repositories must use their
distribution signing infrastructure before they are advertised as official
channels. CI does not yet compare independently rebuilt `.deb`/`.rpm` files,
so package reproducibility is also unverified.

For ordinary desktop installations, prefer the distribution package whenever
one is available. Debian, RPM, and Arch package recipes install the shared
IBus component and its package-owned executable together. This avoids the
source-install flow's administrator-authorized system registration step.
Release archives and `scripts/install-user.sh` remain useful for development
and per-user CLI/GUI installs, but their staged IBus component may still need
system integration before the GNOME IBus route can run. Package availability
and desktop certification are separate: no currently published package has
passed the physical-desktop certification gate.

The earlier Rust 1.93 build failure was resolved by synchronizing the
vendored source branch. The current `1.3.3` daily package built successfully
for amd64 on Resolute and Stonking on 2026-11-07 ([Resolute build
33926583](https://launchpad.net/~cyberducttape/+archive/ubuntu/ppa/+build/33926583),
[Stonking build
33926582](https://launchpad.net/~cyberducttape/+archive/ubuntu/ppa/+build/33926582)).

| Distro | Package | Status | Maintainer |
|--------|---------|--------|------------|
| Arch Linux | `wayexpand` | Build locally | See shared publication status above |
| Ubuntu 24.04 LTS | `.deb` | Release build baseline and package lifecycle CI target | See shared publication status above |
| Ubuntu 26.04 LTS | `wayexpand` | Experimental daily PPA package; `.deb` package lifecycle CI target | See shared publication status above |
| Debian | `.deb` | Build locally from source | See shared publication status above |
| Fedora 45 | `.rpm` | RPM package lifecycle CI target | See shared publication status above |
| aarch64 | `wayexpand` | Build locally from source | See shared publication status above |

The Ubuntu 24.04 and Fedora 45 entries describe package build/install coverage,
not a claim that a stable distribution channel or compositor-certified route
is available. Release publication remains blocked until the production
certification gate passes. GitHub release artifacts carry build-provenance
attestations and checksums; a future APT/Copr repository must use the
repository's signing infrastructure before it is described as an official
package channel.

### Experimental builds

The manually triggered **Experimental build** workflow
(`.github/workflows/experimental.yml`) produces x86_64 and aarch64 archives
plus a `.deb` and `.rpm` from the branch or tag it is dispatched on, without a Rust toolchain on the target
machine. These builds pass the full CI suite but skip real-compositor
certification, so they make no compatibility claim and are not for managed
fleets. By default the workflow only uploads workflow artifacts; with
`publish` enabled it creates a GitHub prerelease tagged
`experimental-<label>` that is never marked as the latest release.

Each build carries its commit in every name:

- binaries report `<version>-dev+<commit>`;
- packages are versioned `<version>+exp.<date>.g<commit>` after an already
  tagged version, or `<version>~exp.<date>.g<commit>` before one, so dpkg and
  rpm always upgrade them to the next certified release;
- file names use `<version>-exp.<date>.g<commit>`.

Verify a download with its `.sha256` file and
`gh attestation verify <file> --repo cyberducttape/wayexpand`.

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
# Replace ${VERSION} with the current release version (e.g., 1.3.3)
tar -xzf wayexpand-${VERSION}-vendored.tar.gz
cd wayexpand-${VERSION}

# Build source package
dpkg-buildpackage -us -uc

# Or build binary package
dpkg-buildpackage -b

# Install locally
sudo dpkg -i ../wayexpand_${VERSION}-1_amd64.deb
```

This documents a local packaging path. Experimental daily packages are
currently published for Ubuntu 26.04 (Resolute) and Ubuntu 26.10 (Stonking),
amd64; check the shared publication status above and the PPA for the latest
package and build state before installing. These daily packages are not
certified releases. The GitHub release workflow only publishes release
artifacts after its certification gates pass. CI also verifies package
installation on Ubuntu 24.04 LTS and 26.04 LTS.

**To maintain:**
1. Update version in `debian/changelog`
2. Run `dch -i` to manage changelog entries
3. Test build: `debuild -us -uc`
4. Push to Launchpad PPA

### Fedora/RHEL

**Status:** The release workflow is configured to attach a local-install RPM
for x86_64 after certification gates pass; no RPM is currently published. The
RPM spec is maintained and built in CI, with package installation checked on
Fedora 45. No official Copr repository is published, so DNF repository-based
updates are not available.

For the release RPM, install the downloaded local package with:

```bash
sudo dnf install ./wayexpand-<version>-1.x86_64.rpm
```

**Local build from spec file:**

```bash
# Prepare for rpmbuild
rpmbuild -ba wayexpand.spec

# Or use mock for clean builds
# Replace ${VERSION} with the current release version (e.g., 1.3.3)
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

The release workflow builds on native x86_64 and aarch64 runners and is
configured to attach both archives after certification gates pass. No release
archive is currently published. For local evaluation on an aarch64 Fedora,
Debian, Ubuntu, or Arch system, build from source after installing Rust 1.92+
and native Wayland dependencies:

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

Experimental daily packages are currently published for Ubuntu 26.04
(Resolute) and Ubuntu 26.10 (Stonking) on amd64. Check the shared publication
status above before installing. To install a supported series:

```bash
sudo add-apt-repository ppa:cyberducttape/ppa
sudo apt update
sudo apt install wayexpand
```

This is a daily development build, not a compositor-certified release. Check
the [PPA page](https://launchpad.net/~cyberducttape/+archive/ubuntu/ppa) for
the latest published version and supported series before installing.

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
archive provides both `rustc` and `cargo` at or above the project MSRV (1.92).
Ubuntu 26.04 Resolute currently satisfies this package's build dependencies.
The latest amd64 builds and published series are listed in the publication
status above. Check Launchpad before advertising any additional series or
architecture as installable.

**Daily builds:** the `main-daily` recipe builds the `launchpad-vendored`
branch once a day when it has changed. Its text is
[`launchpad/wayexpand.recipe`](../launchpad/wayexpand.recipe). The Sync
Launchpad workflow regenerates that branch on every push to `main`; it holds
the `main` tree plus `debian/` (which release archives leave out) and Cargo's
locked vendor sources. To publish it by hand, or to inspect it without pushing:

```bash
GIT_SSH_COMMAND='ssh -F /dev/null' bash scripts/publish-launchpad-branch.sh
bash scripts/publish-launchpad-branch.sh --output /tmp/wayexpand-lp
```

The recipe must point at `launchpad-vendored`, not `main`: a clean `main`
checkout has no `vendor/`, and `debian/rules` stops the build.

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
- **Additional Ubuntu PPA series and architectures** after their build dependencies and Launchpad results are verified

GitHub release artifacts remain gated on certification. The published daily
PPA builds remain experimental; do not treat a successful package build as
desktop certification.

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
