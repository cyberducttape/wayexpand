# Release Promotion Guide

This guide covers the manual steps needed to promote releases on GitHub and manage package repositories.

## Current release promotion

Use this short check for each new stable release. The version-specific history
of earlier promotions belongs in the changelog, not in the active procedure.

### Manual Steps

1. **Open GitHub Releases**
   - Go to https://github.com/cyberducttape/wayexpand/releases

2. **Find the current release**
   - Select the intended stable release for the current version
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
**When:** Before announcing each release

---

## Fedora Copr Repository Publication (Optional)

**Status:** the repository builds through Copr's SCM integration; no official
Copr project is published yet.

Copr builds from the Git repository with the **make_srpm** method:
`.copr/Makefile` runs `scripts/copr-srpm.sh`, which vendors the crates from
the checkout (as the release workflow does) and builds the source RPM, so the
RPM build itself needs no network. Nothing is uploaded by hand.

The build is named from the checkout, so no two builds share an RPM version:

| Checkout | RPM version |
|---|---|
| exactly at the release tag of the spec's `Version` (`X.Y.Z` or `vX.Y.Z`) | `X.Y.Z-1` (stable) |
| N commits after that tag | `X.Y.Z-1.N.gitSHA` (development; upgrades the release, sorts before the next) |
| without that tag | refused |

### One-time setup

1. Create the Copr project with the chroots to publish (for example
   `fedora-43-x86_64`, `fedora-44-x86_64` and their `aarch64` twins).
2. *Packages → New package → SCM*: clone URL
   `https://github.com/cyberducttape/wayexpand.git`, spec `wayexpand.spec`,
   SRPM build method **make_srpm**. Leave *Committish* empty for development
   builds from the default branch, or use a separate package that builds
   release tags.
3. *Settings → Integrations*: add the GitHub webhook so pushes and release
   tags start builds.

### Each release

Pushing the release tag starts the stable build. Check it finishes for every
chroot, then users can `sudo dnf copr enable <owner>/wayexpand`. Before the
first publication, update `fedora_copr_published` in
`packaging/release-status.json` and the README install table.

### Alternative: Community Package

If you prefer not to maintain a Copr repo:
1. Contact Fedora package maintainers (@fedora-packagers)
2. Request wayexpand be added to official Fedora repos
3. Maintainers handle ongoing updates

**Time Required:** 2-3 hours (first time), 30 minutes per release  
**Effort:** Build trigger + monitoring  
**When:** After each release, when Copr publication is part of the release plan

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
  version = "<VERSION>";

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
**When:** When NixOS packaging is included in the release plan

---

## Summary of Release Promotion Tasks

| Task | Priority | Effort | When | Owner |
|------|----------|--------|------|-------|
| Verify current release is latest | Required | 5 min | Each release | Manual (GitHub UI) |
| Fedora Copr | Optional | 2-3 h | Per release plan | Maintainer or volunteer |
| NixOS package | Optional | 2-3 h | Per release plan | Community maintainer preferred |

---

## Release Readiness Checklist

Use this checklist for each release, adapting optional packaging tasks to the
distribution plan:

- [ ] Version, changelog, metadata, and release notes are synchronized
- [ ] Documentation reflects the release's user-facing changes
- [ ] Source archives and checksums are generated and verified
- [ ] Vendored source archive is prepared for offline Debian/Launchpad builds
- [ ] CI artifacts, SBOMs, signatures, and attestations are available
- [ ] Required package builds complete for supported distributions
- [ ] GitHub release is marked as the latest stable release
- [ ] Package repositories are promoted according to the release plan
- [ ] Installation and upgrade instructions are smoke-tested

## Desktop certification gate

Do not promote a compositor backend to a production-supported route based only
on unit tests, a successful protocol probe, or a matcher preview. Before a
release claims desktop support, attach real-session evidence for each claimed
route covering native Wayland and XWayland clients, exact deletion/insertion
with punctuation and Unicode, rapid typing, focus changes during pending
expansions, layout/dead-key/Compose changes, compositor restart and recovery,
password-field handling, and ordinary application shortcuts.

The minimum KDE/KWin matrix is Konsole, Kate, Firefox, Chromium, and a GTK
application. The corresponding GNOME, Sway, and Hyprland routes require their
own sessions and client evidence. Record the compositor version, selected
capture/output backends, layout, client, and whether the result was successful,
failed, partial, or unknown. Until that evidence exists, keep the route marked
experimental or not certified in `docs/SUPPORT_MATRIX.md`.

evdev remains an explicit, experimental compatibility fallback. It has no
sensitive-field signal, does not automatically follow compositor layout
changes, and cannot provide atomic replacement because capture is non-exclusive.
It must not be presented as a universal production fallback.

## Command trust gate

Direct command snippets are shell-free but are still executable programs with
the desktop user's permissions. Process-group cleanup and the daemon service
sandbox are defense in depth, not a complete containment boundary: a trusted
command can detach a child or use any capability still available to its service.
Imported packs strip commands and hotkey actions by default; restoring an
executable snippet is an explicit trust decision. Use named Action Broker
actions only with a reviewed allowlist and hardened broker policy, and do not
treat same-user broker IPC as an OS security boundary.
