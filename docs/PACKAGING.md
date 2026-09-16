# Packaging WayExpand

This guide covers building and maintaining WayExpand packages for different Linux distributions.

## Quick Reference

| Distro | Package | Status | Maintainer |
|--------|---------|--------|------------|
| Arch Linux | `wayexpand` | [AUR](https://aur.archlinux.org) | See below |
| Debian/Ubuntu | `wayexpand` | [PPA](https://launchpad.net) | See below |
| Fedora | `wayexpand` | [Copr](https://copr.fedorainfracloud.org) | See below |

## Building Locally

### Arch Linux (AUR)

```bash
# Clone and build
git clone https://aur.archlinux.org/wayexpand.git
cd wayexpand
makepkg -si
```

**To maintain:**
1. Update `pkgver` and `pkgrel` in `PKGBUILD`
2. Compute SHA256: `sha256sum wayexpand-0.2.0.tar.gz`
3. Update `sha256sums` array
4. Test with `makepkg -si`
5. Push to AUR git repo (requires AUR account)

### Debian/Ubuntu

```bash
# Build source package
dpkg-buildpackage -us -uc

# Or build binary package
dpkg-buildpackage -b

# Install locally
sudo dpkg -i ../wayexpand_0.2.0-1_amd64.deb
```

**To maintain:**
1. Update version in `debian/changelog`
2. Run `dch -i` to manage changelog entries
3. Test build: `debuild -us -uc`
4. Push to Launchpad PPA

### Fedora/RHEL

```bash
# Prepare for rpmbuild
rpmbuild -ba wayexpand.spec

# Or use mock for clean builds
mock wayexpand-0.2.0-1.fc39.src.rpm
```

**To maintain:**
1. Update version in `wayexpand.spec`
2. Add changelog entry in `%changelog` section
3. Test build: `rpmbuild -ba wayexpand.spec`
4. Push to Copr (if using Copr) or submit to Fedora package collection

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
git commit -m "Update to v0.2.0"
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
dput ppa:username/wayexpand ../wayexpand_0.2.0-1_source.changes
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

### Test AUR
```bash
# In a clean chroot
archiso-mount-rw
pacman -S wayexpand
wayexpand-gui
```

### Test Debian
```bash
# In a container
docker run -it debian:bookworm bash
# Add PPA and install
add-apt-repository ppa:itchyitchy123/wayexpand
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

1. **Tag in git:** `git tag v0.2.0 && git push origin v0.2.0`
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

## References

- [ArchWiki: Creating packages](https://wiki.archlinux.org/title/Creating_packages)
- [Debian New Maintainers' Guide](https://www.debian.org/doc/manuals/maint-guide/)
- [Fedora Package Maintenance Guide](https://docs.fedoraproject.org/en-US/package-maintainers/)
