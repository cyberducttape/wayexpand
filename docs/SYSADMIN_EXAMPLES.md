# System Administration Examples

WayExpand is particularly useful for system administrators who frequently need to type complex commands, configurations, and deployment scripts. This guide showcases production-ready snippets for common sysadmin tasks.

## Certificate and SSL Management

### Generate Self-Signed Certificate
```toml
[[expansion]]
trigger = ".ssl-cert"
replacement = "sudo openssl req -x509 -nodes -days 365 -newkey rsa:2048 -keyout /etc/ssl/private/wayexpand.key -out /etc/ssl/certs/wayexpand.crt"
description = "Generate self-signed SSL certificate (365 days)"
tags = ["ssl", "security", "certificate"]
category = "Security"
enabled = true
```

### Check Certificate Expiration
```toml
[[expansion]]
trigger = ".cert-check"
replacement = "openssl x509 -in /etc/ssl/certs/wayexpand.crt -text -noout | grep -E 'Not Before|Not After|Subject:'"
description = "Check SSL certificate expiration date and details"
tags = ["ssl", "certificate", "security"]
category = "Security"
enabled = true
```

### Renew Let's Encrypt Certificate
```toml
[[expansion]]
trigger = ".letsencrypt"
replacement = "sudo certbot renew --quiet && sudo systemctl reload nginx"
description = "Renew Let's Encrypt certificate and reload nginx"
tags = ["ssl", "letsencrypt", "tls"]
category = "Security"
enabled = true
```

## Log Management

### Configure Logrotate
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
enabled = true
```

### View Recent Logs
```toml
[[expansion]]
trigger = ".logs"
replacement = "sudo journalctl -u wayexpand -n 50 --no-pager"
description = "Show last 50 systemd journal entries for WayExpand"
tags = ["logging", "diagnostics"]
category = "System"
enabled = true
```

### Monitor Log in Real-Time
```toml
[[expansion]]
trigger = ".tail"
replacement = "sudo tail -f /var/log/wayexpand/daemon.log"
description = "Stream daemon log output (Ctrl+C to exit)"
tags = ["logging", "monitoring"]
category = "System"
enabled = true
```

## Backup and Restore

### Backup Configuration
```toml
[[expansion]]
trigger = ".backup"
replacement = "sudo tar -czf /backup/wayexpand-$(date +%Y%m%d).tar.gz /home/$USER/.config/wayexpand/ && echo 'Backup complete'"
description = "Backup WayExpand configuration to /backup"
tags = ["backup", "configuration"]
category = "Maintenance"
enabled = true
```

### Backup Database
```toml
[[expansion]]
trigger = ".db-backup"
replacement = "sudo pg_dump -U postgres wayexpand_db | gzip > /backup/wayexpand_db_$(date +%Y%m%d_%H%M%S).sql.gz"
description = "Backup PostgreSQL database with timestamp"
tags = ["database", "backup"]
category = "Database"
enabled = true
```

### List Recent Backups
```toml
[[expansion]]
trigger = ".backups"
replacement = "ls -lh /backup/ | grep wayexpand"
description = "List recent WayExpand backups"
tags = ["backup", "diagnostics"]
category = "Maintenance"
enabled = true
```

## Systemd Service Management

### Create Systemd User Service
```toml
[[expansion]]
trigger = ".systemd"
replacement = """[Unit]
Description=WayExpand Text Expansion Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/wayexpand daemon
Restart=on-failure
RestartSec=10

[Install]
WantedBy=graphical-session.target"""
description = "Systemd user service configuration for WayExpand"
tags = ["systemd", "service"]
category = "System"
enabled = true
```

### Check Service Status
```toml
[[expansion]]
trigger = ".status"
replacement = "sudo systemctl status wayexpand --full && echo '---' && ps aux | grep wayexpand | grep -v grep"
description = "Check daemon status and process information"
tags = ["monitoring", "diagnostics"]
category = "System"
enabled = true
```

### Restart Service
```toml
[[expansion]]
trigger = ".restart"
replacement = "sudo systemctl restart wayexpand && sleep 2 && systemctl status wayexpand"
description = "Restart daemon and show status (2s delay for startup)"
tags = ["service", "deployment"]
category = "System"
enabled = true
```

## Firewall Rules (UFW)

### Enable UFW and SSH
```toml
[[expansion]]
trigger = ".ufw-ssh"
replacement = "sudo ufw default deny incoming && sudo ufw default allow outgoing && sudo ufw allow 22/tcp && sudo ufw enable"
description = "Configure UFW firewall with SSH access"
tags = ["firewall", "security"]
category = "Network"
enabled = true
```

### Allow Specific Subnet
```toml
[[expansion]]
trigger = ".ufw-subnet"
replacement = "sudo ufw allow from 192.168.1.0/24 to any port 22 && sudo ufw allow from 192.168.1.0/24 to any port 80 && sudo ufw allow from 192.168.1.0/24 to any port 443"
description = "Allow network requests from specific subnet (SSH, HTTP, HTTPS)"
tags = ["firewall", "network"]
category = "Network"
enabled = true
```

## Docker Container Management

### View Container Logs
```toml
[[expansion]]
trigger = ".docker-logs"
replacement = "docker logs --timestamps --tail 100 -f {{cursor}}"
description = "View Docker container logs with timestamps (place cursor in container name)"
tags = ["docker", "logging"]
category = "Containers"
enabled = true
```

### Docker System Cleanup
```toml
[[expansion]]
trigger = ".docker-prune"
replacement = "docker system prune -a --volumes"
description = "Remove unused Docker images, containers, volumes, and networks"
tags = ["docker", "cleanup"]
category = "Containers"
enabled = true
```

### Docker Compose Restart
```toml
[[expansion]]
trigger = ".docker-restart"
replacement = "docker-compose -f /etc/wayexpand/docker-compose.yml down && docker-compose -f /etc/wayexpand/docker-compose.yml up -d"
description = "Restart Docker Compose services"
tags = ["docker", "deployment"]
category = "Containers"
enabled = true
```

## Deployment and Release

### Deploy New Binary
```toml
[[expansion]]
trigger = ".deploy"
replacement = "sudo systemctl stop wayexpand && sudo cp /tmp/wayexpand-release /usr/local/bin/wayexpand && sudo systemctl start wayexpand && systemctl status wayexpand"
description = "Deploy new WayExpand binary (stop, replace, start, verify)"
tags = ["deployment", "release"]
category = "Maintenance"
enabled = true
```

### Create Release Tag
```toml
[[expansion]]
trigger = ".release"
replacement = "git tag -a v{{cursor}} -m 'Release version {{cursor}}' && git push origin v{{cursor}}"
description = "Create and push git release tag (edit version numbers)"
tags = ["git", "release"]
category = "Development"
enabled = true
```

### Build and Deploy
```toml
[[expansion]]
trigger = ".build-deploy"
replacement = "cargo build --release -p wayexpand-gui && sudo cp target/release/wayexpand-gui /usr/local/bin/ && sudo systemctl restart wayexpand"
description = "Build release binary and deploy GUI"
tags = ["deployment", "build"]
category = "Development"
enabled = true
```

## User and Permissions Management

### Add User to Group
```toml
[[expansion]]
trigger = ".usergroup"
replacement = "sudo usermod -aG wayexpand $USER && echo 'Added $USER to wayexpand group. Log out and back in for changes to take effect.'"
description = "Add current user to wayexpand group"
tags = ["users", "permissions"]
category = "Security"
enabled = true
```

### Check File Permissions
```toml
[[expansion]]
trigger = ".perms"
replacement = "stat /etc/wayexpand/expansions.toml | grep -E 'Access:|Uid:|Gid:'"
description = "Display file permissions, owner, and group"
tags = ["permissions", "diagnostics"]
category = "Security"
enabled = true
```

## System Information and Monitoring

### View System Load and Memory
```toml
[[expansion]]
trigger = ".sysload"
replacement = "uptime && echo '---' && free -h && echo '---' && df -h"
description = "Show uptime, memory usage, and disk space"
tags = ["monitoring", "diagnostics"]
category = "System"
enabled = true
```

### Network Diagnostics
```toml
[[expansion]]
trigger = ".netstat"
replacement = "sudo ss -tlnp | grep wayexpand"
description = "Show listening sockets for WayExpand daemon"
tags = ["network", "diagnostics"]
category = "Network"
enabled = true
```

### Find Large Files
```toml
[[expansion]]
trigger = ".large-files"
replacement = "find / -type f -size +1G 2>/dev/null | head -20"
description = "Find files larger than 1GB on system (top 20)"
tags = ["maintenance", "diagnostics"]
category = "System"
enabled = true
```

## Nginx Configuration Snippets

### Reverse Proxy Configuration
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
enabled = true
```

### SSL Configuration
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
enabled = true
```

## Adding These Snippets to Your Config

Copy the TOML expansion blocks into your `~/.config/wayexpand/expansions.toml` file, or use the GUI to create them:

```bash
# View your current config location
wayexpand config --show

# Or edit directly with your editor
$EDITOR ~/.config/wayexpand/expansions.toml
```

The GUI provides a visual editor with live preview for all snippets. Category filtering helps organize the library for quick access during daily work.

## Tips for Effective Sysadmin Snippets

1. **Use meaningful triggers**: `.ssl-cert` is clearer than `.sc`
2. **Prefix by category**: `.ufw-*`, `.docker-*`, `.backup-*`
3. **Add helpful descriptions**: "Generate self-signed SSL certificate (365 days)"
4. **Use `{{cursor}}`**: For interactive placeholders in commands
5. **Tag by topic**: `["ssl", "security"]` enables filtering
6. **Set `category`**: Organize by system area (Security, Network, Database, etc.)
7. **Test before relying**: Always test in a non-production environment first
8. **Document prerequisites**: "Requires sudo access", "Uses PostgreSQL", etc.

## Production Considerations

- **Security**: These snippets often require elevated privileges. Use with caution in shared environments.
- **Idempotency**: Many sysadmin tasks should be idempotent (safe to run multiple times)
- **Auditing**: Consider logging snippet usage for compliance in regulated environments
- **Backup before deploy**: Always test destructive operations in a test environment first
- **Service restart delays**: Some snippets use `sleep` to allow services time to start

---

**Note**: These examples are templates. Adjust paths, ports, users, and other parameters to match your specific environment before using in production.
