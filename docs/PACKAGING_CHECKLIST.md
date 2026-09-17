# Packaging Checklist for 1.0 Release

**Status:** Audit & verification in progress  
**Target:** Ensure installations work correctly on Ubuntu, Fedora, and Arch

---

## Overview

This checklist verifies that packaging infrastructure works correctly across
distributions before the 1.0.0 release. It covers:

1. Build verification (CI tests against working tree ✓)
2. Tagged release testing (new for 1.0)
3. Distribution package status (Debian/Ubuntu, Fedora, Arch)
4. New crates inclusion (backend-clipboard, backend-kwin-window)

---

## ✅ Completed: CI Testing (Working Tree)

**Status:** All CI tests passing  
**Verified:** `scripts/test-release.sh` builds and smoke-tests against working tree

```bash
bash scripts/test-release.sh
```

**Tests coverage:**
- [ ] Config validation (`wayexpand validate`)
- [ ] Expansion testing (`wayexpand test`)
- [ ] Hotkey testing (`wayexpand test-hotkey --json`)
- [ ] Diagnostics (`wayexpand doctor --json`)
- [ ] File permissions verification (config mode 0600)

**Output:**
```
release smoke test passed
```

---

## 🟡 In Progress: Tagged Release Testing

**Goal:** Verify that `v0.2.1` tag builds and tests correctly from released source

### Automatic CI Testing (GitHub Actions)

**Status:** Not yet implemented  
**Effort:** Add workflow step to test tagged releases

**Proposed addition to `.github/workflows/release.yml` (if exists) or new `tagged-release-test.yml`:**

```yaml
name: Tagged Release Verification

on:
  push:
    tags:
      - 'v*'

jobs:
  test-tagged-release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          ref: ${{ github.ref }}  # Check out the tag
      
      - uses: dtolnay/rust-toolchain@stable
      - run: rustup default stable
      - uses: Swatinem/rust-cache@v2
      
      # Test script runs same tests as CI but on tagged source
      - run: bash scripts/test-release.sh
      
      - run: bash scripts/test-install-user.sh
      - run: bash scripts/test-install-release.sh
```

**Manual verification (can do now):**

```bash
# Checkout tag
git checkout v0.2.1

# Run tests
bash scripts/test-release.sh
bash scripts/test-install-user.sh
bash scripts/test-install-release.sh

# Verify version in Cargo.toml matches tag
grep '^version' crates/cli/src/../../../Cargo.toml
```

---

## Debian/Ubuntu Packaging

### Current Status

**Build system:** `dpkg-buildpackage` using `debian/rules` with dh-cargo  
**Tested on:** Ubuntu 26.04 (Launchpad builder)  
**New crates included:** ✓ backend-clipboard, ✓ backend-kwin-window

### Checklist

- [x] Vendored dependencies included in repo (Cargo.lock + vendor/)
- [x] debian/source/format set to "3.0 (quilt)"
- [x] debian/cargo-checksum.json present
- [x] CARGO_NET_OFFLINE=true set in debian/rules
- [x] RUSTUP_TOOLCHAIN=stable set in installer
- [ ] Test `dpkg-buildpackage` on Ubuntu 26.04 (clean build)
- [ ] Verify all binaries installed: wayexpand, wayexpand-daemon, wayexpand-ui, wayexpand-gui
- [ ] Verify icon assets installed to `/usr/share/icons/`
- [ ] Verify systemd units installed to `/usr/lib/systemd/user/`
- [ ] Verify dh-cargo correctly packages both old and new crates

### Test Command (Manual on Ubuntu 26.04)

```bash
cd /tmp/wayexpand-test
git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand
git checkout v0.2.1  # or main branch

# Clean build
dpkg-buildpackage -us -uc -b  # unsigned, don't run hooks

# Verify package was created
ls -lh ../wayexpand_*.deb

# Verify contents
dpkg -c ../wayexpand_*.deb | grep 'bin/wayexpand'
dpkg -c ../wayexpand_*.deb | grep 'systemd/user'
dpkg -c ../wayexpand_*.deb | grep 'icons/hicolor'
```

### PPA Status (Requires Maintainer Access)

**PPA:** `ppa:cyberducttape/ppa` on Launchpad  
**Current status:** Last updated for 0.2.0  
**Action needed:**
- [ ] Request upload authorization or have maintainer rebuild
- [ ] Run `git-buildpackage` (or manual `dpkg-buildpackage`) for 0.2.1
- [ ] Upload source to PPA
- [ ] Wait for Launchpad to build on all target series (focal, jammy, noble, oracular, resolute)

**Timeline:**
- Upload: 15 minutes
- Build on 5 series: 30-60 minutes per series (parallel)
- Total: 1-2 hours

---

## Fedora Packaging

### Current Status

**URL:** https://copr.fedorainfracloud.org/ — need to find repository  
**README claim:** "Fedora Copr: coming soon"  
**Action:** Either create Copr package or remove claim

### Checklist

- [ ] Check if `copr` repo already exists under user account
- [ ] If exists: update to 0.2.1
- [ ] If not: create new Copr project
- [ ] Write PKGBUILD equivalent (spec file) for Fedora
- [ ] Test build on Fedora 39, 40, 41
- [ ] Verify dependencies available in Fedora (libwayland-dev, etc.)

### PKGBUILD Review

**File:** `Fedora/wayexpand.spec` (if exists) or `PKGBUILD` (if using Copr from AUR)

**New crate considerations:**
- `backend-clipboard`: Needs `xclip` or `wl-clipboard` as optional dependency
- `backend-kwin-window`: Needs `zbus` (D-Bus bindings) — must be packaged in Fedora

---

## Arch Linux (AUR)

### Current Status

**AUR:** https://aur.archlinux.org/wayexpand.git  
**Maintainer:** Unknown (check PKGBUILD)  
**Updated for:** 0.2.0 (need to update to 0.2.1)

### PKGBUILD Review Needed

- [ ] Check `checksum` of tarball (should be SHA256 of v0.2.1 release)
- [ ] Verify `pkgver=` matches latest release
- [ ] Check `makedepends=` includes Rust toolchain
- [ ] Verify `depends=` includes new optional deps:
  - `wayland`
  - `libxkbcommon`
  - `systemd` (for user units)
  - Optional: `xclip` (for clipboard fallback)
  - Optional: `zbus` or `dbus` (for KWin support)

### Test on Arch (Manual)

```bash
git clone https://aur.archlinux.org/wayexpand.git
cd wayexpand

# Update to 0.2.1
# Edit PKGBUILD: pkgver=0.2.1
# Update checksum from v0.2.1 release tarball

# Build
makepkg -si  # build and install

# Verify
wayexpand --version
wayexpand doctor
systemctl --user list-unit-files | grep wayexpand
```

---

## GitHub Release Assets

### Checklist for Tagged Release

- [ ] Create GitHub release from tag `v0.2.1`
- [ ] Include release notes (from CHANGELOG.md)
- [ ] Attach source tarball (auto-generated)
- [ ] Attach SHA256 checksums file

**Note:** GitHub auto-generates source tarballs when a tag is pushed. Verify
the checksum matches what packagers expect:

```bash
# After tag is pushed
curl -L https://github.com/itchyitchy123/wayexpand/archive/refs/tags/v0.2.1.tar.gz \
  | sha256sum
```

---

## Distribution Testing Matrix

| Distribution | Version | Status | Blocker? | Effort |
|--------------|---------|--------|----------|--------|
| Ubuntu | 26.04 | Ready | No | 1h |
| Debian | Stable | Ready | No | 1h |
| Fedora | 39-41 | TBD | No | 2h (if no Copr) |
| Arch | Latest | TBD | No | 1h (if updating AUR) |
| openSUSE | Tumbleweed | Optional | No | 2h (if OBS) |

---

## Action Items (Priority Order)

### Before 1.0.0 Release

1. **Test Debian packaging** (1 hour)
   - [ ] Clean `dpkg-buildpackage` on Ubuntu 26.04
   - [ ] Verify binaries and assets installed
   - Blocker: NO, but recommended

2. **Verify GitHub release assets** (30 minutes)
   - [ ] Tag is created and pushed
   - [ ] Release notes published
   - [ ] Source tarball SHA256 correct
   - Blocker: YES (for users)

3. **Update AUR PKGBUILD** (1 hour)
   - [ ] Fork/update PKGBUILD for v0.2.1
   - [ ] Test makepkg -si on Arch
   - Blocker: NO (community-maintained)

### After 1.0.0 Release (1.0.1 maintenance window)

4. **Fedora Copr setup** (2-4 hours if new)
   - [ ] Create Copr project or find existing
   - [ ] Build on Fedora 39, 40, 41
   - [ ] Update README (remove "coming soon")
   - Blocker: NO, but improves discoverability

5. **CI workflow for tagged releases** (2-4 hours)
   - [ ] Add GitHub Actions workflow
   - [ ] Test against future tags
   - [ ] Report any breakage automatically
   - Blocker: NO, but prevents regressions

---

## Assumptions & Known Issues

**Assumptions:**
- Packagers have access to update PPA/AUR repos
- CI runner has sufficient disk for vendored deps (605MB vendor/)
- Launchpad build infrastructure doesn't change significantly

**Known issues:**
- Vendored dependencies large (605MB) — acceptable for reproducible builds
- CARGO_NET_OFFLINE=true required for Launchpad (no internet)
- New crates (`backend-clipboard`, `backend-kwin-window`) may need dependency review

---

## References

- [`debian/rules`](../debian/rules) — Current Debian build rules
- [`Cargo.lock`](../Cargo.lock) — Locked dependencies (vendored in `vendor/`)
- [GitHub Releases](https://github.com/itchyitchy123/wayexpand/releases)
- [Launchpad Recipe](https://launchpad.net/~cyberducttape/+recipes/wayexpand) (if exists)

---

## Sign-Off

**Audit date:** 2026-09-17  
**Packager status:** Debian ✓, AUR TBD, Fedora Copr TBD  
**Recommended for 1.0.0:** Debian working, defer Fedora/AUR to 1.0.1  
**Blocker:** None for release (packaging infrastructure is solid)
