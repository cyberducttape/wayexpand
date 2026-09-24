# Packaging WayExpand

This guide covers building and maintaining WayExpand packages for different Linux distributions.

## Rust toolchain policy

WayExpand supports Rust **1.87 or newer**. This is the project MSRV and is
declared in the workspace `Cargo.toml`; CI checks Rust 1.87 and the pinned
project toolchain in `rust-toolchain.toml` (currently Rust 1.96.0).
Distribution packages must provide at least `rustc 1.87` and `cargo 1.87`.
Older Debian/Ubuntu releases may need a maintained Rust toolchain from the
distribution backports or an isolated toolchain installation.

## Quick Reference

| Distro | Package | Status | Maintainer |
|--------|---------|--------|------------|
| Arch Linux | `wayexpand` | Packaging preview | Not yet submitted to AUR; x86_64 only |
| Ubuntu | `wayexpand` | [PPA](https://launchpad.net) | Official (cyberducttape/ppa) |
| Debian | source/release build | No native archive yet | Use vendored source archive for package builds |
| Fedora/RHEL | `wayexpand` | Build from source | ⚠️ No official Copr yet |
| aarch64 | source build | No pre-built release archive yet | Cross-build or build natively |

## Building Locally

### Arch Linux (packaging preview)

```bash
# Clone and build
git clone https://github.com/itchyitchy123/wayexpand.git
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

The Launchpad PPA path targets Ubuntu series. Plain Debian users should build
from the vendored release archive or install from the upstream release/source
workflow until a native Debian repository exists.

**To maintain:**
1. Update version in `debian/changelog`
2. Run `dch -i` to manage changelog entries
3. Test build: `debuild -us -uc`
4. Push to Launchpad PPA

### Fedora/RHEL

**Status:** No official Copr repository yet. Build locally or from source.

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
git clone https://github.com/itchyitchy123/wayexpand
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

The GitHub release workflow currently publishes a pre-built Linux archive only
for `x86_64`; there is no official aarch64 binary to download yet. On an
aarch64 Fedora, Debian, Ubuntu, or Arch system, build from the source archive
or checkout after installing Rust 1.87+ and the native Wayland dependencies:

```bash
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev
cargo build --locked --release --workspace
```

The binaries are in `target/release/`. Run `wayexpand doctor` before enabling
a user service. Native builds are preferred over cross-builds because the
Wayland and compositor protocol libraries must match the target system.

For a cross-build, install a Rust target and target-native development
libraries first, then use Cargo's normal `--target aarch64-unknown-linux-gnu`
flow. WayExpand does not currently publish a prebuilt sysroot or cross-build
toolchain.

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

**First Time:**
1. Create Launchpad account at https://launchpad.net
2. Create PPA: Settings → Personal Package Archives → Create new PPA
3. Generate GPG key if needed: `gpg --gen-key`
4. Upload source package via `dput`

**Updates:**
```bash
# Build source package
debuild -S -sa

# Upload to PPA (replace ${VERSION} with current version)
dput ppa:cyberducttape/ppa ../wayexpand_${VERSION}-1_source.changes
```

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
# Ubuntu PPA test
add-apt-repository ppa:cyberducttape/ppa
apt update && apt install wayexpand
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

Consider setting up:
- **GitHub Actions** to auto-publish releases when tags are pushed
- **Copr webhook** to auto-rebuild when an official Copr exists
- **Ubuntu PPA** to auto-sync from GitHub releases

This minimizes manual work for patch releases.

---

## Vendored Dependencies

The working repository does not need to keep `vendor/` checked in. Normal
online builds use `Cargo.lock` and fetch crates from the configured Cargo
registry.

Release tooling supports two source archive shapes:

- `wayexpand-<version>.tar.gz`: clean source archive without `vendor/`
- `wayexpand-<version>-vendored.tar.gz`: offline-build archive with `vendor/`
  and the generated Cargo source replacement config

Use the clean archive for build systems that can access Cargo registries or run
`cargo vendor` during their build step. Use the vendored archive for Launchpad,
air-gapped builders, or any policy that requires all Rust dependencies to be
present in the source upload. Large local `vendor/` directories are build
artifacts, not required repository content.

---

## References

- [ArchWiki: Creating packages](https://wiki.archlinux.org/title/Creating_packages)
- [Debian New Maintainers' Guide](https://www.debian.org/doc/manuals/maint-guide/)
- [Fedora Package Maintenance Guide](https://docs.fedoraproject.org/en-US/package-maintainers/)
