# Upgrading WayExpand

This guide covers upgrading from one version of WayExpand to another, including breaking changes, data migration, and rollback procedures.

## Quick Summary

**Safe to upgrade:** Your snippets (configuration) are never touched. Configuration format is stable across 1.x releases.

**How to upgrade:**

```bash
# If installed via install-user.sh
cd wayexpand
git pull
./scripts/install-user.sh

# If installed via PPA
sudo apt update && sudo apt upgrade wayexpand

# If built from the repository PKGBUILD preview
git pull && makepkg -si
```

The daemon will reload automatically. No service restart required.

---

## Version-Specific Upgrade Notes

### Upgrading to v1.2.x from v1.0.x or v1.1.x

**No breaking changes.** Your configuration is compatible.

**New features (optional):**
- `{{cursor}}` placement marker in templates
- Date math: `{{date+3d}}`, `{{time-5h}}`, etc.
- `propagate_case` for automatic case preservation
- Undo last expansion with configurable hotkey
- `app_filter` improvements (see below)

**App-filter changes:**
- Filtering logic now prefers `app_id` over window title (safer, more predictable)
- Existing filters continue to work, but may match differently
- **Action:** Review app-filtered snippets after upgrade if you rely on app_filter

**Example:** If you had a filter `["thunderbird"]` matching window titles, it now only matches the actual Thunderbird app, not other windows with "Thunderbird" in their title.

**Sensitive field detection:**
- evdev backend still has no password-field detection (unchanged from v1.1.x)
- input-method-v2 backend continues to suspend matching in password fields

**Daemon restart:**
- Not required — daemon will reload on upgrade
- If using manual systemd units, `systemctl --user restart wayexpand-input-method.service` to apply immediately

### Upgrading to v1.1.x from v1.0.x

**No breaking changes.** Your configuration is compatible.

**GUI enhancements only:**
- New color packs (retro terminal themes, high contrast)
- Font scaling (0.8x-2.0x)
- German language support with in-app switching
- Better keyboard focus indicators

No daemon behavior changes.

---

## Configuration Backups

### Before Upgrading

Back up your current configuration:

```bash
cp ~/.config/wayexpand/expansions.toml ~/wayexpand-backup-$(date +%Y%m%d).toml
```

### After Upgrade (If Something Breaks)

Restore from backup:

```bash
cp ~/wayexpand-backup-20260918.toml ~/.config/wayexpand/expansions.toml
systemctl --user restart wayexpand-input-method.service
```

### Manual Backup

The supported backup procedure is the command shown above. The GUI does not
currently provide a File → Export action; do not rely on that menu path.

---

## Troubleshooting Upgrades

### Daemon won't start after upgrade

Check the journal for errors:

```bash
journalctl --user -u wayexpand-input-method.service -n 20
```

**Common issues:**

1. **Configuration validation failed**
   - Your TOML has a syntax error
   - Check: `wayexpand validate ~/.config/wayexpand/expansions.toml`
   - Restore backup if validation fails

2. **Missing backend**
   - You upgraded but your systemd service points to an unavailable backend
   - Solution: Run `wayexpand doctor` and enable the correct service
   - Example: `systemctl --user enable --now wayexpand-input-method.service`

3. **Socket permission error**
   - Old socket file is still present
   - Solution: Remove it and restart: `rm $XDG_RUNTIME_DIR/wayexpand.sock && systemctl --user restart wayexpand-input-method.service`

### Expansions not working after upgrade

1. Check daemon is running: `systemctl --user status wayexpand-input-method.service`
2. Check capture readiness: `wayexpand doctor`
3. Check configuration loaded: `wayexpand list ~/.config/wayexpand/expansions.toml`
4. Check logs: `journalctl --user -u wayexpand-input-method.service -f`

### GUI shows old settings after upgrade

Restart the GUI: `pkill -f wayexpand-gui && wayexpand-gui`

---

## Downgrading (Rollback)

If an upgrade causes issues, downgrade to the previous version:

### From Source

```bash
git checkout v1.1.1
./scripts/install-user.sh
systemctl --user restart wayexpand-input-method.service
```

### From PPA

```bash
sudo apt install wayexpand=1.1.1-1
sudo apt-mark hold wayexpand  # Prevent auto-upgrade
```

To resume auto-updates: `sudo apt-mark unhold wayexpand`

### Future AUR publication

```bash
# Check available versions
# Once published, check available versions with:
# yay -S wayexpand --show

# Downgrade to specific version (requires git history)
cd /tmp && git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand
git checkout v1.1.1
makepkg -si
```

---

## Breaking Changes (By Version)

### v1.0.0 (First Stable Release)

- CLI exit codes are now stable and guaranteed
- JSON output schema is now stable and guaranteed
- Configuration format is now stable (no breaking changes in 1.x)

**If upgrading from v0.2.x:** Configuration format is compatible. No migration needed.

### Future Versions (v2.0+)

Breaking changes will be announced in release notes with migration guides. None are planned for v1.x.

---

## Multi-System Upgrades (Fleet Deployment)

If managing WayExpand across multiple machines:

### Test on one machine first

```bash
# On test system
./scripts/install-user.sh
wayexpand doctor
# Test your snippets manually
```

### Roll out gradually

1. **Pilot group:** 10% of machines
2. **Monitor:** Check `journalctl` for errors across pilot group
3. **Gradual:** 25%, then 50%, then 100%
4. **Rollback plan:** Keep previous version available

### Automated fleet upgrades

Via configuration management (Ansible, Puppet, etc.):

```bash
#!/bin/bash
set -e

# Backup
cp ~/.config/wayexpand/expansions.toml ~/wayexpand-backup.toml

# Upgrade
cd ~/wayexpand
git pull
./scripts/install-user.sh

# Verify
wayexpand validate ~/.config/wayexpand/expansions.toml || {
  # Restore backup if validation fails
  cp ~/wayexpand-backup.toml ~/.config/wayexpand/expansions.toml
  exit 1
}

# Restart daemon
systemctl --user restart wayexpand-input-method.service

# Health check
sleep 2
systemctl --user is-active wayexpand-input-method.service
```

---

## Getting Help

- Check [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) for known issues
- Run `wayexpand doctor` for diagnostic information
- Check logs: `journalctl --user -u wayexpand-input-method.service`
- Open an issue on GitHub with your version and error logs

---

## Version History

| Version | Release Date | Major Changes |
|---------|--------------|---------------|
| 1.1.2 | **Unreleased** | Security hardening (P0 fixes), diagnostics improvements |
| 1.1.1 | 2026-09-18 | GUI themes, font scaling, sysadmin examples |
| 1.1.0 | 2026-09-18 | Language support, color packs, date math, cursor placement |
| 1.0.0 | 2026-09-17 | **First stable release** — stability guarantees |
| 0.2.0 | 2026-09-16 | evdev backend, GUI redesign |
| 0.1.0 | 2026-09-10 | Initial release |

---

## Stability Guarantees

See [COMPATIBILITY.md](COMPATIBILITY.md) for detailed stability guarantees across versions. In summary:

- 🔒 **Stable:** Configuration format, CLI exit codes, JSON output schema
- 🟡 **Experimental:** Desktop backend support (varies by compositor)
- ⚠️ **Unstable:** Internal APIs (changes without notice)

Upgrades within 1.x are always safe for configuration. Backend behavior may change (documented in release notes).
