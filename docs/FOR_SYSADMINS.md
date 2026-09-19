# WayExpand for System Administrators

This guide covers deploying, managing, and supporting WayExpand in team and enterprise environments.

## Deployment Models

### Individual User (Self-Service)

Users install locally from PPA/AUR/source:

```bash
# Ubuntu/Debian
sudo apt install wayexpand

# Arch
yay -S wayexpand

# From source
./scripts/install-user.sh --enable --service=wayexpand-input-method.service
```

**Admin overhead:** Minimal. Users manage their own configs.

**Support burden:** Help users choose correct backend (`wayexpand doctor`), troubleshoot compositor issues.

### Managed Deployment (Fleet)

For teams with shared machines or controlled environments:

**Approach 1: Package distribution** (recommended)
- Deploy via standard package managers (APT, AUR, Copr)
- Users self-install from organizational repo
- Admin manages package version, not individual instances

**Approach 2: Centralized binary distribution**
- Build wayexpand once in CI
- Distribute prebuilt binaries to all machines
- Users run installer from shared location
- Admin controls which version is current

**Approach 3: Configuration management** (for many machines)
- Use Ansible, Puppet, or similar
- Automate installation + configuration
- Centralized snippet library via git/distribution
- Fully auditable deployment

## Installation at Scale

### Ansible Playbook Example

```yaml
---
- name: Deploy WayExpand
  hosts: workstations
  tasks:
    - name: Add PPA (Ubuntu/Debian)
      ansible.builtin.apt_repository:
        repo: "ppa:cyberducttape/ppa"
      when: ansible_os_family == "Debian"

    - name: Install WayExpand
      ansible.builtin.apt:
        name: wayexpand
        state: present
      when: ansible_os_family == "Debian"

    - name: Create config directory
      ansible.builtin.file:
        path: "{{ ansible_user_dir }}/.config/wayexpand"
        state: directory
        mode: "0700"

    - name: Deploy snippet library
      ansible.builtin.copy:
        src: expansions.toml
        dest: "{{ ansible_user_dir }}/.config/wayexpand/expansions.toml"
        owner: "{{ ansible_user_id }}"
        mode: "0600"

    - name: Enable systemd user service
      ansible.builtin.systemd:
        name: wayexpand-input-method.service
        state: started
        enabled: yes
        scope: user
```

### Pre-Deployment Checklist

- [ ] Verify Wayland availability: `echo $WAYLAND_DISPLAY`
- [ ] Check compositor: `echo $XDG_CURRENT_DESKTOP`
- [ ] Verify systemd --user works: `systemctl --user status`
- [ ] Test on pilot group (10% of machines) before full rollout
- [ ] Have rollback plan (previous wayexpand version available)

## Configuration Management

### Shared Snippet Library

**Option 1: Git-based distribution**
```bash
# Each team has a repo with expansions.toml
git clone https://internal-git/team-wayexpand-snippets
cp team-wayexpand-snippets/expansions.toml ~/.config/wayexpand/
```

**Option 2: Config management tool** (Ansible, Puppet)
```
roles/wayexpand/files/expansions.toml
```

**Option 3: Central file server**
```bash
cp /mnt/shared-config/wayexpand/expansions.toml ~/.config/wayexpand/
```

### Snippet Library Audit

Periodically audit deployed snippets for:
- Credentials/secrets (should use password manager instead)
- Personally identifiable information (PII)
- Outdated contact information or URLs
- Performance issues (command timeouts)

```bash
# Find all snippets with program= (commands)
grep -r "program =" ~/.config/wayexpand/

# Validate syntax
wayexpand validate ~/.config/wayexpand/expansions.toml

# List all triggers
wayexpand list ~/.config/wayexpand/expansions.toml
```

## Troubleshooting at Scale

### Inventory & Health Checks

For managed deployments, create a health check script:

```bash
#!/bin/bash
# wayexpand-health-check.sh
set -e

echo "=== WayExpand Health Check ==="

# Version
echo "Version: $(wayexpand --version)"

# Daemon status
systemctl --user is-active wayexpand-input-method.service && \
  echo "✓ Service: running" || \
  echo "✗ Service: NOT running"

# Configuration validation
wayexpand validate ~/.config/wayexpand/expansions.toml && \
  echo "✓ Configuration: valid" || \
  echo "✗ Configuration: INVALID"

# Backend diagnostics
echo ""
echo "=== Diagnostics ==="
wayexpand doctor

# JSON for scripting
wayexpand doctor --json
```

Deploy via cron or configuration management to collect fleet status:

```bash
# Run on all machines, collect results
ansible all -m script -a wayexpand-health-check.sh
```

### Common Issues

**Daemon not starting:**
```bash
journalctl --user -u wayexpand-input-method.service -n 50
wayexpand doctor
```

**Service stuck reconnecting:**
- Compositor restarted or crashed
- Wayland protocol issue
- `systemctl --user restart wayexpand-input-method.service`

**Configuration won't reload:**
```bash
wayexpand validate ~/.config/wayexpand/expansions.toml
# Fix reported errors, then:
wayexpand reload
```

## Security Considerations

### Snippet Library Security

1. **Access control:** Config files are mode 0600 (readable only by owner)
2. **Audit:** Review who has commit access to shared snippet repos
3. **Secrets management:** Never store credentials in snippets
   - Use password manager for credentials
   - Use environment variables for API keys
   - Use `{{username}}` and `{{hostname}}` instead of hardcoding
4. **Evdev permissions:** If using `--source=evdev`:
   - Explicit root step (`install-evdev-permissions.sh`)
   - Requires active consent
   - Document that users are granting `input` group membership

### Systemd Sandbox Constraints

Commands in `program=` expansions run under systemd sandbox:

```ini
ProtectSystem=strict      # Read-only /
ProtectHome=read-only     # Read-only $HOME
RestrictAddressFamilies=AF_UNIX  # No network
```

Commands that work manually may fail in expansions if they need write access.

**Validation:** Test with `wayexpand-gui` Preview button, check logs:
```bash
journalctl --user -u wayexpand-input-method.service -f
# Type a trigger with a program, observe result
```

## Scaling Considerations

### Performance

- **Matcher:** Scales linearly with snippet count (trie-based, bounded at 10,000 snippets)
- **Reload:** Parses and validates full config before swap (safe but may stall on very large configs)
- **Commands:** Bounded to 5-second timeout, max 1 MiB output (safe)
- **Network:** No dependencies, all local

### Monitoring

Collect from journalctl:
```bash
journalctl --user -u wayexpand-input-method.service -o json \
  | grep -E '(ERROR|WARNING|FATAL)' \
  | jq '{timestamp, message, priority}'
```

Alert on:
- `state=error` (configuration error, won't recover without admin action)
- Repeated restarts (check `/var/log/apt/`, recent changes)
- `config_state=error` (malformed config)

### Capacity Planning

Per-machine resources:
- **Memory:** ~50 MB baseline + 1–5 MB per 1,000 snippets
- **Disk:** ~100 KB per 100 snippets in config file
- **CPU:** Negligible (<0.1% idle, <1% during typing)
- **Network:** None (all local)

No central server required.

## Key Paths

| Purpose | Path | Owner | Mode |
|---------|------|-------|------|
| Config | `~/.config/wayexpand/expansions.toml` | User | 0600 |
| State | `$XDG_RUNTIME_DIR/wayexpand.sock` | User | 0600 |
| Systemd unit | `~/.config/systemd/user/wayexpand-*.service` | User | 0644 |
| Example config | `/etc/wayexpand/expansions.toml.example` | Root | 0644 |
| Man pages | `/usr/share/man/man1/wayexpand.1` | Root | 0644 |

## Support Resources

- **User documentation:** `/usr/share/doc/wayexpand/` or `docs/` in repo
- **Troubleshooting:** [docs/wiki/Troubleshooting.md](wiki/Troubleshooting.md)
- **Compatibility:** [docs/SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)
- **Security:** [SECURITY.md](../SECURITY.md)
- **Changelog:** [CHANGELOG.md](../CHANGELOG.md)

## Feedback & Contributions

- **Issues:** https://github.com/itchyitchy123/wayexpand/issues
- **Discussions:** https://github.com/itchyitchy123/wayexpand/discussions
- **Pull Requests:** https://github.com/itchyitchy123/wayexpand/pulls

---

**Last updated:** 2026-09-19
**Scope:** WayExpand v1.1.2+
