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
**Publication status from the shared manifest (audited 2026-11-18):** GitHub experimental-preview release 1.3.6 binary assets published: True. Launchpad PPA: experimental daily builds for ['amd64'].

| Distribution | PPA package status |
|---|---|
| Ubuntu 26.04 (Resolute) | 1.3.6 — current |
| Ubuntu 26.10 (Stonking) | 1.3.6 — current |
| Ubuntu 24.04 (Noble) | 0.2.0 — stale |
| Ubuntu 22.04 (Jammy) | 0.2.0 — stale |

Fedora Copr published: False; Arch AUR submitted: False. Package CI targets: Ubuntu 24.04, Ubuntu 26.04, Fedora 45. Physical desktop certification: False. Previous-package upgrade CI passed: True.

| Upgrade scenario | Scope | Status | Expected behavior |
|---|---|---|---|
| previous-release-to-current | package-manager | passed | Candidate package installs over previous release; user configuration and snippets remain valid. |
| user-customization-preserved | package-manager | passed | User-owned snippets and custom fields survive schema migration; after migration, configuration bytes remain unchanged across rollback, re-upgrade, reinstall, and uninstall/reinstall. |
| uninstall-reinstall | package-manager | passed | Package-owned executables are removed and reinstall restores package components without replacing user configuration. |
| running-daemon-activated | live-user-session | not-tested | Affected process is restarted and reports the candidate build; no stale daemon remains. |
| gnome-ibus-upgrade | physical-gnome-session | not-tested | IBus registration and GNOME source remain usable or setup presents a precise recovery step. |
| interrupted-package-upgrade | package-manager | not-tested | Previous working package remains usable or package repair restores a coherent installation. |
<!-- generated-install-status:end -->

The release payload is built on Ubuntu 24.04. The package lifecycle job uses a
1.3.2 package fixture to exercise upgrade, rollback, re-upgrade, reinstall, and
removal on Ubuntu 24.04, Ubuntu 26.04, and Fedora 45. It now preserves and
validates a user-customized configuration through those package operations.
The exact current scenario status is shown in the generated matrix below. This
container test accepts the supported stable-ID migration when validating the
older fixture, checks that its built-in and user-owned snippets survive, then
requires byte-for-byte preservation through later package operations. It
checks CLI, daemon, GUI, and UI binary versions after upgrade. After rollback
it checks that the installed binaries link and that the CLI and daemon report
1.3.2; the 1.3.2 GUI and UI have no `--version` option.
It does not simulate a package-manager interruption, confirm that a running
user daemon was replaced, or exercise an active GNOME IBus source; those
scenarios remain untested in the matrix. GitHub
release artifacts receive build-provenance attestations and checksums;
repositories must use their distribution signing infrastructure before they
are advertised as official channels. CI does not yet compare independently
rebuilt `.deb`/`.rpm` files, so package reproducibility is also unverified.

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
vendored source branch. On 2026-11-07, a `1.3.3` daily package built
successfully for amd64 on Resolute and Stonking ([Resolute build
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
3. The release workflow publishes `wayexpand-<version>.PKGBUILD` with the
   SHA256 checksum of the archive it built and the release tag in `source`;
   use that file for the AUR
4. Test with `makepkg -si`
5. Before publication, generate `.SRCINFO` and verify in a clean Arch chroot

### Ubuntu PPA / Debian Vendored Build

```bash
# Debian packaging expects the vendored release archive for offline builds.
# Do not build the clean Git checkout: Launchpad builders cannot fetch crates.io.
# Replace ${VERSION} with the release version you are building.
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
certified releases. GitHub releases are published with binaries once CI
passes and are marked latest; without a successful physical-session
certification their notes label them an experimental preview. CI also verifies
package installation on Ubuntu 24.04 LTS and 26.04 LTS.

**To maintain:**
1. Update version in `debian/changelog`
2. Run `dch -i` to manage changelog entries
3. Test build: `debuild -us -uc`
4. Push to Launchpad PPA

### Fedora/RHEL

**Status:** The release workflow attaches a local-install RPM
for x86_64 to every GitHub release; no RPM is currently published. The
RPM spec is maintained and built in CI, with package installation checked on
Fedora 45. No official Copr repository is published, so DNF repository-based
updates are not available.

For the release RPM, install the downloaded local package with:

```bash
sudo dnf install ./wayexpand-<version>-1.x86_64.rpm
```

**Local build from the repository:** the spec builds offline from the
vendored source archive (`Source0`), which a Git checkout does not contain.
`.copr/Makefile` produces it from the checkout, as the release workflow does,
and wraps it in a source RPM:

```bash
git clone https://github.com/cyberducttape/wayexpand
cd wayexpand
make -f .copr/Makefile srpm outdir=/tmp/wayexpand-srpm
rpmbuild --rebuild /tmp/wayexpand-srpm/wayexpand-*.src.rpm   # or: mock <srpm>
```

**Setting up the official Copr repository** (maintainer, one time):

1. Sign in at https://copr.fedorainfracloud.org and create a project, for
   example `wayexpand`, with the chroots to publish (`fedora-43-x86_64`,
   `fedora-44-x86_64`, `fedora-45-x86_64` (the CI package target), and their
   `aarch64` twins). Fedora 43 ships Rust
   1.99, above the required 1.92.
2. *Packages → New package → SCM*: clone URL
   `https://github.com/cyberducttape/wayexpand.git`, spec file
   `wayexpand.spec`, SRPM build method **make_srpm**. Leave *Committish*
   empty to build the default branch, or set a release tag.
3. *Settings → Integrations*: add the GitHub webhook, so pushing a release
   tag rebuilds the package. Build from release tags only; a build from
   `main` carries the version of the last release.
4. Once a build succeeds, set `fedora_copr_published` in
   `packaging/release-status.json` and add the `dnf copr enable` line to
   the README.

`make_srpm` runs `.copr/Makefile` as root in a network-enabled chroot; it
installs `cargo`, `git` and `rpm-build` if missing and vendors crates there.
The RPM build itself runs without network access.

`scripts/copr-srpm.sh` names each build so that no two builds share an RPM
version. A checkout exactly at the release tag of the spec's `Version`
(`X.Y.Z` or `vX.Y.Z`) builds `X.Y.Z-1`. Any later commit builds
`X.Y.Z-1.N.gitSHA`, where `N` counts the commits since the tag: it upgrades
the release, sorts before the next release, grows along `main`, and names
the source commit. A checkout without that tag is refused.
`scripts/test-copr-srpm-version.sh` checks this, including RPM ordering
when `rpmdev-vercmp` is available.

### aarch64

The [latest GitHub release](https://github.com/cyberducttape/wayexpand/releases/latest)
includes an aarch64 binary archive and checksum. No aarch64 `.deb` or `.rpm`
package is published; install the archive for your user:

```bash
sha256sum --check wayexpand-*-linux-aarch64.tar.gz.sha256
tar -xzf wayexpand-*-linux-aarch64.tar.gz
./wayexpand-*-linux-aarch64/scripts/install-release.sh
```

For a custom build on an aarch64 Fedora, Debian, Ubuntu, or Arch system,
install Rust 1.92+ and native Wayland dependencies, then build from source:

```bash
# Install native dependencies for your distribution
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev   # Debian, Ubuntu
sudo dnf install gcc pkgconf-pkg-config wayland-devel libxkbcommon-devel    # Fedora
sudo pacman -S --needed base-devel wayland libxkbcommon                     # Arch
cargo build --locked --release --workspace
```

The binaries are in `target/release/`. Run `./target/release/wayexpand doctor`
before enabling a user service. Native builds are preferred over cross-builds
because the Wayland and compositor protocol libraries must match the target
system.

CI uses native aarch64 hardware, avoiding fragile cross-builds of the Wayland
and compositor-protocol stack. Binary availability does not establish
compositor certification; check the certification matrix before relying on a
desktop route.

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
source_version="<SOURCE_VERSION_FROM_DSC>" # copy the exact version shown in the .dsc filename
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

Launchpad does not report failed builds, so the Launchpad Builds workflow
(`.github/workflows/launchpad-builds.yml`) checks the PPA's public build
records every night at 23:40 UTC, after the daily recipe has built. It fails
when the newest finished build of any series still being built did not
succeed, and prints the failing lines of its build log. Series with no build
within 14 days of the newest one (jammy, noble) count as retired and are
listed but not judged. To check by hand:

```bash
python3 scripts/check-launchpad-builds.py
```

**Note:** See [RELEASING.md](RELEASING.md) for the authoritative release version workflow
(it is the single source of truth for version numbers across all distributions).

### Fedora/Copr

Copr builds the package itself through `.copr/Makefile` (see
[Fedora/RHEL](#fedorarhel) for the one-time setup). With the webhook in
place, pushing the release tag starts the build; nothing is uploaded by hand.
`scripts/prepare-release.sh` keeps the spec's `Version` in step with the tag.

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
