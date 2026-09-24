# Packaging WayExpand

This guide covers building and maintaining WayExpand packages for different Linux distributions.

## Rust toolchain policy

WayExpand supports Rust **1.87 or newer**. This is the project MSRV and is
declared in the workspace `Cargo.toml`; CI checks both Rust 1.87 and stable.
Distribution packages must provide at least `rustc 1.87` and `cargo 1.87`.
Older Debian/Ubuntu releases may need a maintained Rust toolchain from the
distribution backports or an isolated toolchain installation.

## Quick Reference

| Distro | Package | Status | Maintainer |
|--------|---------|--------|------------|
| Arch Linux | `wayexpand` | Packaging preview | Not yet submitted to AUR; x86_64 only |
| Debian/Ubuntu | `wayexpand` | [PPA](https://launchpad.net) | Official (cyberducttape/ppa) |
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
1. Update `pkgver` and `pkgrel` in `PKGBUILD`
2. Compute SHA256: `sha256sum wayexpand-1.0.0.tar.gz`
3. Update `sha256sums` array
4. Test with `makepkg -si`
5. Before publication, generate `.SRCINFO` and verify in a clean Arch chroot

### Debian/Ubuntu

```bash
# Build source package
dpkg-buildpackage -us -uc

# Or build binary package
dpkg-buildpackage -b

# Install locally
sudo dpkg -i ../wayexpand_1.0.0-1_amd64.deb
```

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
mock wayexpand-1.1.1-1.fc39.src.rpm
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
4. Copy `PKGBUILD`, `.gitignore`, `.SRCINFO` to repo
5. Generate `.SRCINFO`: `makepkg --printsrcinfo > .SRCINFO`
6. Commit and push

**Updates:**
```bash
cd wayexpand-aur
# Update PKGBUILD with new version
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "Update to v1.0.0"
git push
```

### Debian/Ubuntu PPA

**First Time:**
1. Create Launchpad account at https://launchpad.net
2. Create PPA: Settings → Personal Package Archives → Create new PPA
3. Generate GPG key if needed: `gpg --gen-key`
4. Upload source package via `dput`

**Updates:**
```bash
# Build source package
debuild -S -sa

# Upload to PPA
dput ppa:cyberducttape/ppa ../wayexpand_1.1.1-1_source.changes
```

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
# Add PPA and install
add-apt-repository ppa:cyberducttape/ppa
apt update && apt install wayexpand
wayexpand-gui
```

### Test Fedora
```bash
# In a container
docker run -it fedora:39 bash
# Enable Copr and install
dnf copr enable @itchyitchy123/wayexpand
dnf install wayexpand
wayexpand-gui
```

---

## Versioning and Release Flow

When releasing a new version:

1. **Tag in git:** `git tag v1.0.0 && git push origin v1.0.0`
2. **Update all packaging files:**
   - `PKGBUILD`: bump `pkgver`, reset `pkgrel=1`
   - `debian/changelog`: add new entry (use `dch -i`)
   - `wayexpand.spec`: bump `Version:`, reset `Release: 1%{?dist}`
3. **Build locally and test on each distro**
4. **Submit/upload to each distro** (see above)
5. **Announce release** on GitHub, Reddit, etc.

---

## Automated Updates

Consider setting up:
- **GitHub Actions** to auto-publish releases when tags are pushed
- **Copr webhook** to auto-rebuild when repository updates
- **Debian PPA** to auto-sync from GitHub releases

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
