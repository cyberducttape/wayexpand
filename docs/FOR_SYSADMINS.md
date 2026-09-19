# WayExpand for System Administrators

**Navigation:** [Home](../README.md) > **System Administration**

---

This guide covers deploying, managing, and supporting WayExpand in team and enterprise environments.

## Deployment Models

### Individual User (Self-Service)

Users install locally via available package managers or source:

```bash
# Ubuntu/Debian (via PPA)
sudo apt install wayexpand

# Arch (packaging prepared; build locally)
git clone https://github.com/itchyitchy123/wayexpand.git
cd wayexpand && makepkg -si

# From source
./scripts/install-user.sh --enable --service=wayexpand-input-method.service
```

**Packaging Status:**
- ✅ Ubuntu/Debian: Available via PPA
- 📦 Arch: PKGBUILD prepared (not yet official AUR submission)
- 📦 Fedora: Copr packaging prepared (not yet published)
- 🔧 Others: Build from source using `./scripts/install-user.sh`

**Admin overhead:** Minimal. Users manage their own configs.

**Support burden:** Help users choose correct backend (`wayexpand doctor`), troubleshoot compositor issues.

### Managed Deployment (Fleet)

For teams with shared machines or controlled environments:

**Approach 1: Package distribution** (recommended)
- **Ubuntu/Debian:** Deploy via PPA (apt)
- **Arch:** Packaging prepared (not yet official AUR)
- **Fedora:** Packaging prepared (Copr packaging ready)
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

## Ready-Made Snippet Examples

WayExpand is particularly useful for system administrators who frequently need to type complex commands and configurations. Below are production-ready snippets for common sysadmin tasks.

### Certificate and SSL Management

**Generate Self-Signed Certificate**
```toml
[[expansion]]
trigger = ".ssl-cert"
replacement = "sudo openssl req -x509 -nodes -days 365 -newkey rsa:2048 -keyout /etc/ssl/private/wayexpand.key -out /etc/ssl/certs/wayexpand.crt"
description = "Generate self-signed SSL certificate (365 days)"
tags = ["ssl", "security", "certificate"]
category = "Security"
```

**Check Certificate Expiration**
```toml
[[expansion]]
trigger = ".cert-check"
replacement = "openssl x509 -in /etc/ssl/certs/wayexpand.crt -text -noout | grep -E 'Not Before|Not After|Subject:'"
description = "Check SSL certificate expiration date and details"
tags = ["ssl", "certificate", "security"]
category = "Security"
```

**Renew Let's Encrypt Certificate**
```toml
[[expansion]]
trigger = ".letsencrypt"
replacement = "sudo certbot renew --quiet && sudo systemctl reload nginx"
description = "Renew Let's Encrypt certificate and reload nginx"
tags = ["ssl", "letsencrypt", "tls"]
category = "Security"
```

### Log Management

**Configure Logrotate**
```toml
[[expansion]]
trigger = ".logrotate"
replacement = """/var/log/wayexpand/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 wayexpand wayexpand
    sharedscripts
}"""
description = "Logrotate configuration for daemon logs"
tags = ["logging", "maintenance"]
category = "System"
```

**View Recent Logs**
```toml
[[expansion]]
trigger = ".logs"
replacement = "sudo journalctl -u wayexpand -n 50 --no-pager"
description = "Show last 50 systemd journal entries for WayExpand"
tags = ["logging", "diagnostics"]
category = "System"
```

**Monitor Log in Real-Time**
```toml
[[expansion]]
trigger = ".tail"
replacement = "sudo tail -f /var/log/wayexpand/daemon.log"
description = "Stream daemon log output (Ctrl+C to exit)"
tags = ["logging", "monitoring"]
category = "System"
```

### Backup and Restore

**Backup Configuration**
```toml
[[expansion]]
trigger = ".backup"
replacement = "sudo tar -czf /backup/wayexpand-$(date +%Y%m%d).tar.gz /home/$USER/.config/wayexpand/ && echo 'Backup complete'"
description = "Backup WayExpand configuration to /backup"
tags = ["backup", "configuration"]
category = "Maintenance"
```

**Backup Database**
```toml
[[expansion]]
trigger = ".db-backup"
replacement = "sudo pg_dump -U postgres wayexpand_db | gzip > /backup/wayexpand_db_$(date +%Y%m%d_%H%M%S).sql.gz"
description = "Backup PostgreSQL database with timestamp"
tags = ["database", "backup"]
category = "Database"
```

**List Recent Backups**
```toml
[[expansion]]
trigger = ".backups"
replacement = "ls -lh /backup/ | grep wayexpand"
description = "List recent WayExpand backups"
tags = ["backup", "diagnostics"]
category = "Maintenance"
```

### Systemd Service Management

**Create Systemd User Service**
```toml
[[expansion]]
trigger = ".systemd"
replacement = """[Unit]
Description=WayExpand Text Expansion Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/wayexpand-daemon
Restart=on-failure
RestartSec=10

[Install]
WantedBy=graphical-session.target"""
description = "Systemd user service configuration for WayExpand"
tags = ["systemd", "service"]
category = "System"
```

**Check Service Status**
```toml
[[expansion]]
trigger = ".status"
replacement = "sudo systemctl status wayexpand --full && echo '---' && ps aux | grep wayexpand | grep -v grep"
description = "Check daemon status and process information"
tags = ["monitoring", "diagnostics"]
category = "System"
```

**Restart Service**
```toml
[[expansion]]
trigger = ".restart"
replacement = "sudo systemctl restart wayexpand && sleep 2 && systemctl status wayexpand"
description = "Restart daemon and show status (2s delay for startup)"
tags = ["service", "deployment"]
category = "System"
```

### Firewall Rules (UFW)

**Enable UFW and SSH**
```toml
[[expansion]]
trigger = ".ufw-ssh"
replacement = "sudo ufw default deny incoming && sudo ufw default allow outgoing && sudo ufw allow 22/tcp && sudo ufw enable"
description = "Configure UFW firewall with SSH access"
tags = ["firewall", "security"]
category = "Network"
```

**Allow Specific Subnet**
```toml
[[expansion]]
trigger = ".ufw-subnet"
replacement = "sudo ufw allow from 192.168.1.0/24 to any port 22 && sudo ufw allow from 192.168.1.0/24 to any port 80 && sudo ufw allow from 192.168.1.0/24 to any port 443"
description = "Allow network requests from specific subnet (SSH, HTTP, HTTPS)"
tags = ["firewall", "network"]
category = "Network"
```

### Docker Container Management

**View Container Logs**
```toml
[[expansion]]
trigger = ".docker-logs"
replacement = "docker logs --timestamps --tail 100 -f {{cursor}}"
description = "View Docker container logs with timestamps (place cursor in container name)"
tags = ["docker", "logging"]
category = "Containers"
```

**Docker System Cleanup**
```toml
[[expansion]]
trigger = ".docker-prune"
replacement = "docker system prune -a --volumes"
description = "Remove unused Docker images, containers, volumes, and networks"
tags = ["docker", "cleanup"]
category = "Containers"
```

**Docker Compose Restart**
```toml
[[expansion]]
trigger = ".docker-restart"
replacement = "docker-compose -f /etc/wayexpand/docker-compose.yml down && docker-compose -f /etc/wayexpand/docker-compose.yml up -d"
description = "Restart Docker Compose services"
tags = ["docker", "deployment"]
category = "Containers"
```

### Deployment and Release

**Deploy New Binary**
```toml
[[expansion]]
trigger = ".deploy"
replacement = "sudo systemctl stop wayexpand && sudo cp /tmp/wayexpand-release /usr/local/bin/wayexpand && sudo systemctl start wayexpand && systemctl status wayexpand"
description = "Deploy new WayExpand binary (stop, replace, start, verify)"
tags = ["deployment", "release"]
category = "Maintenance"
```

**Create Release Tag**
```toml
[[expansion]]
trigger = ".release"
replacement = "git tag -a v{{cursor}} -m 'Release version {{cursor}}' && git push origin v{{cursor}}"
description = "Create and push git release tag (edit version numbers)"
tags = ["git", "release"]
category = "Development"
```

**Build and Deploy**
```toml
[[expansion]]
trigger = ".build-deploy"
replacement = "cargo build --release -p wayexpand-gui && sudo cp target/release/wayexpand-gui /usr/local/bin/ && sudo systemctl restart wayexpand"
description = "Build release binary and deploy GUI"
tags = ["deployment", "build"]
category = "Development"
```

### User and Permissions Management

**Add User to Group**
```toml
[[expansion]]
trigger = ".usergroup"
replacement = "sudo usermod -aG wayexpand $USER && echo 'Added $USER to wayexpand group. Log out and back in for changes to take effect.'"
description = "Add current user to wayexpand group"
tags = ["users", "permissions"]
category = "Security"
```

**Check File Permissions**
```toml
[[expansion]]
trigger = ".perms"
replacement = "stat /etc/wayexpand/expansions.toml | grep -E 'Access:|Uid:|Gid:'"
description = "Display file permissions, owner, and group"
tags = ["permissions", "diagnostics"]
category = "Security"
```

### System Information and Monitoring

**View System Load and Memory**
```toml
[[expansion]]
trigger = ".sysload"
replacement = "uptime && echo '---' && free -h && echo '---' && df -h"
description = "Show uptime, memory usage, and disk space"
tags = ["monitoring", "diagnostics"]
category = "System"
```

**Network Diagnostics**
```toml
[[expansion]]
trigger = ".netstat"
replacement = "sudo ss -tlnp | grep wayexpand"
description = "Show listening sockets for WayExpand daemon"
tags = ["network", "diagnostics"]
category = "Network"
```

**Find Large Files**
```toml
[[expansion]]
trigger = ".large-files"
replacement = "find / -type f -size +1G 2>/dev/null | head -20"
description = "Find files larger than 1GB on system (top 20)"
tags = ["maintenance", "diagnostics"]
category = "System"
```

### Nginx Configuration Snippets

**Reverse Proxy Configuration**
```toml
[[expansion]]
trigger = ".nginx-proxy"
replacement = """location / {
    proxy_pass http://localhost:3000;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}"""
description = "Nginx reverse proxy configuration"
tags = ["nginx", "web"]
category = "Web"
```

**SSL Configuration**
```toml
[[expansion]]
trigger = ".nginx-ssl"
replacement = """listen 443 ssl http2;
ssl_certificate /etc/ssl/certs/wayexpand.crt;
ssl_certificate_key /etc/ssl/private/wayexpand.key;
ssl_protocols TLSv1.2 TLSv1.3;
ssl_ciphers HIGH:!aNULL:!MD5;"""
description = "Nginx SSL/TLS configuration"
tags = ["nginx", "ssl"]
category = "Web"
```

### Using These Snippets

Copy the TOML expansion blocks into your `~/.config/wayexpand/expansions.toml` file, or use the GUI to create them:

```bash
# View your current config location
wayexpand config --show

# Or edit directly with your editor
$EDITOR ~/.config/wayexpand/expansions.toml
```

The GUI provides a visual editor with live preview for all snippets. Category filtering helps organize the library for quick access during daily work.

**Tips for effective sysadmin snippets:**
1. Use meaningful triggers: `.ssl-cert` is clearer than `.sc`
2. Prefix by category: `.ufw-*`, `.docker-*`, `.backup-*`
3. Add helpful descriptions: "Generate self-signed SSL certificate (365 days)"
4. Use `{{cursor}}`: For interactive placeholders in commands
5. Tag by topic: `["ssl", "security"]` enables filtering
6. Set `category`: Organize by system area (Security, Network, Database, etc.)
7. Test before relying: Always test in a non-production environment first
8. Document prerequisites: "Requires sudo access", "Uses PostgreSQL", etc.

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
| Man page | `/usr/share/man/man1/wayexpand.1` (distribution packages) or `~/.local/share/man/man1/wayexpand.1` (user installer) | Package/user | 0644 |

## Production Considerations

**Security:** These snippets often require elevated privileges. Use with caution in shared environments.

**Idempotency:** Many sysadmin tasks should be idempotent (safe to run multiple times). Test before deploying at scale.

**Auditing:** Consider logging snippet usage for compliance in regulated environments.

**Backup before deploy:** Always test destructive operations in a test environment first.

**Service restart delays:** Some snippets use `sleep` to allow services time to start.

**Note:** The examples above are templates. Adjust paths, ports, users, and other parameters to match your specific environment before using in production.

## Support Resources

- **User documentation:** `/usr/share/doc/wayexpand/` or `docs/` in repo
- **Configuration:** [docs/GETTING_STARTED.md](GETTING_STARTED.md)
- **Compatibility:** [docs/SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)
- **Enterprise deployment:** [docs/ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md), [docs/PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md)
- **Security:** [docs/SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md)
- **Changelog:** [CHANGELOG.md](../CHANGELOG.md)

## Feedback & Contributions

- **Issues:** https://github.com/itchyitchy123/wayexpand/issues
- **Discussions:** https://github.com/itchyitchy123/wayexpand/discussions
- **Pull Requests:** https://github.com/itchyitchy123/wayexpand/pulls

---

**Last updated:** 2026-09-19
**Scope:** WayExpand v1.1.2+
