# Release Promotion Guide

This guide covers the manual steps needed to promote releases on GitHub and manage package repositories.

## v1.1.2 promotion (superseded)

The original v1.1.2 promotion task is no longer actionable: GitHub's current
`/releases/latest` URL resolves to the newer v1.2 release. There is no reason
to mark v1.1.2 latest retroactively. Use the same five-minute check whenever a
new release is published.

**Root Cause:** GitHub's `/releases/latest` resolves to whichever release was marked "Set as the latest release" in the UI.

### Manual Steps

1. **Open GitHub Releases**
   - Go to https://github.com/cyberducttape/wayexpand/releases

2. **Find the current release**
   - Select the newest intended stable release
   - Click the three-dot menu icon next to the release title

3. **Promote to Latest**
   - Click "Edit release"
   - Check the box: "Set as the latest release"
   - Click "Update release"

4. **Verify**
   - Visit https://github.com/cyberducttape/wayexpand/releases/latest
   - Confirm it redirects to the intended release tag

### Why This Matters

- Package managers and installation scripts check `/releases/latest`
- Users following default links get the newest stable version
- Outdated defaults harm adoption and support (users on old versions)

**Time Required:** 5 minutes  
**Effort:** Manual UI actions only  
**When:** Today (before any new release announcements)

---

## Fedora Copr Repository Publication (Optional - 2-3 hours)

**Status:** PKGBUILD and wayexpand.spec prepared, not yet published to Copr.

### Prerequisites

- Fedora account with Copr authorization
- `copr-cli` installed locally: `sudo dnf install copr-cli`
- Copr project "wayexpand" already created (or create new one)

### Publication Steps

1. **Configure copr-cli**
   ```bash
   copr-cli config
   # Enter your Copr credentials when prompted
   ```

2. **Build from GitHub**
   ```bash
   copr-cli build \
     --nowait \
     --enable-net=on \
     wayexpand \
     "https://github.com/cyberducttape/wayexpand/archive/v1.1.2.tar.gz"
   ```

3. **Monitor Build**
   - Visit https://copr.fedorainfracloud.org/coprs/yourusername/wayexpand/
   - Verify builds complete for all architectures (x86_64, aarch64, etc.)

4. **Enable Repository**
   - In Copr project settings, mark repo as "enable by default"
   - Users can then install: `sudo dnf copr enable yourusername/wayexpand`

### Alternative: Community Package

If you prefer not to maintain a Copr repo:
1. Contact Fedora package maintainers (@fedora-packagers)
2. Request wayexpand be added to official Fedora repos
3. Maintainers handle ongoing updates

**Time Required:** 2-3 hours (first time), 30 minutes per release  
**Effort:** Build trigger + monitoring  
**When:** After v1.2.0 release (next priority)

---

## NixOS Package (Optional - Community Contribution)

**Status:** No NixOS package exists. Popular request from Nix community.

### Two Approaches

**Approach A: Community-Maintained (Recommended)**
1. File issue requesting NixOS maintainer
2. NixOS maintainers take ownership
3. Automatic updates via nixpkgs CI

**Approach B: Self-Maintained**
1. Submit PR to nixpkgs with package recipe
2. Package maintains update responsibility
3. ~2-3 hours to write and test Nix expression

### NixOS Package Template

```nix
{ lib, rustPlatform, pkg-config, libwayland, libxkbcommon }:

rustPlatform.buildRustPackage rec {
  pname = "wayexpand";
  version = "1.1.2";

  src = fetchFromGitHub {
    owner = "cyberducttape";
    repo = "wayexpand";
    rev = "v${version}";
    sha256 = "sha256-xxxxxxxxxxxxx="; # Run nix flake update to fill this
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ libwayland libxkbcommon ];

  postInstall = ''
    install -Dm644 desktop/wayexpand.desktop $out/share/applications/
    install -Dm644 systemd/wayexpand*.service $out/share/systemd/user/
  '';

  meta = with lib; {
    description = "Text expansion for Wayland";
    homepage = "https://github.com/cyberducttape/wayexpand";
    license = licenses.mit;
    maintainers = with maintainers; [ /* your nixpkgs maintainer ID */ ];
    platforms = platforms.linux;
  };
}
```

**Time Required:** 2-3 hours (one-time setup)  
**When:** After broader package manager coverage (Q4)

---

## Summary of Release Promotion Tasks

| Task | Priority | Effort | When | Owner |
|------|----------|--------|------|-------|
| Verify current release is latest | ✅ Superseded by v1.2 | 5 min | Each release | Manual (GitHub UI) |
| Fedora Copr | 🟡 High | 2-3 h | This week | Stephan or volunteer |
| NixOS package | 🟢 Medium | 2-3 h | Q4 | Community maintainer preferred |

---

## Checklist Before v1.2.0 Release

- [x] Current release promotion verified for v1.2; repeat for the next release
- [ ] SBOM files generated and published (automated in CI)
- [ ] Binary signatures/attestations verified (automated in CI)
- [ ] Fedora Copr published (manual, optional for v1.2.0)
- [ ] Release notes written (auto-generated by GitHub)
- [ ] Documentation updated for new features
- [ ] CHANGELOG.md reflects all user-facing changes
