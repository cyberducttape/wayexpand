# Organization Policy Reference

**Navigation:** [Home](../README.md) > [System Administration](FOR_SYSADMINS.md) > **Organization Policy**

---

> **Status:** Core feature implemented as of v1.1.1

Organization policies enforce administrator-defined constraints on text expansions, protecting sensitive environments and preventing unsafe operations.

## Policy File Location

Policies are loaded from `/etc/wayexpand/policy.toml` (root-owned, strictly validated)
by the daemon, CLI diagnostics, and the IBus service. An invalid or insecure
policy is a startup/diagnostic failure rather than a silent fallback.

## Security Validation

Policy files must meet strict security requirements:
- Regular file (not symlink)
- Root ownership (`uid = 0`)
- Permissions: `0600` (rw-------), `0400` (r--------), or `0440` (r--r-----)
  - `0600`: Root read/write only (most restrictive)
  - `0400`: Root read-only (secure, immutable after deployment)
  - `0440`: Root read-only, group-readable (allows unprivileged services in the policy group to read)
- **No world-accessible bits** (world must not be able to read)
- No world-writable bits ever allowed
- Maximum size: 100 KB (DoS prevention)
- Parent directory `/etc/wayexpand/` also validated (root-owned, not group/world-writable)

## Policy Fields

```toml
[organization]
# Enforcement mode: true = block violations, false = warn only
safe_mode = false

# Disable all command execution (expansions with commands are blocked)
disable_commands = false

# Disable hotkey execution (hotkeys parse but don't run)
disable_hotkeys = false

# Require command/hotkey programs to be absolute paths
require_absolute_commands = false

# Disable title-based fallback in app filtering (require app_id match only)
disable_title_matching = false

# Maximum replacement text size in bytes (0 = unlimited)
max_replacement_size = 65536

# Allowed output backends (empty = all allowed)
allowed_backends = ["libei", "input-method-v2"]

# Allowed curated packs from ~/.local/share/wayexpand/packs/
allowed_packs = ["approved-pack-1", "approved-pack-2"]

# Audit prefix for violation logging to journald
audit_prefix = "wayexpand-policy"
```

## Modes

### Safe Mode (Enforcement)
When `safe_mode = true`:
- Policy violations **prevent** expansions from executing
- Violations logged as **errors** to journald
- Suitable for locked-down production environments
- Blocks: commands, hotkeys, title matching, oversized replacements, disallowed backends
- Can require absolute command paths so `program = "git"` cannot resolve
  differently based on the daemon's `PATH`

### Audit Mode (Logging Only)
When `safe_mode = false`:
- Policy violations **allow** expansions to proceed
- Violations logged as **warnings** to journald
- Suitable for testing policies before full enforcement
- Enables discovery of problematic snippets without breaking workflows

## Examples

### Example 1: Locked-Down Environment
```toml
[organization]
safe_mode = true
disable_commands = true
disable_hotkeys = true
require_absolute_commands = true
disable_title_matching = false
max_replacement_size = 1024
allowed_backends = ["input-method-v2"]
allowed_packs = []
audit_prefix = "corp-policy"
```

### Example 2: SRE-Friendly Policy
```toml
[organization]
safe_mode = true
disable_commands = false
disable_hotkeys = false
require_absolute_commands = true
disable_title_matching = false
max_replacement_size = 65536
allowed_backends = ["libei", "input-method-v2"]
allowed_packs = ["sre-tools", "infrastructure-commands"]
audit_prefix = "sre-policy"
```

### Example 3: Audit-Only (Testing)
```toml
[organization]
safe_mode = false
disable_commands = true
disable_hotkeys = false
require_absolute_commands = false
disable_title_matching = false
max_replacement_size = 65536
allowed_backends = []
allowed_packs = []
audit_prefix = "test-policy"
```

## Deployment

### With Ansible
See [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) for fleet configuration.

### With Puppet
See [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) for fleet configuration.

### Manual Deployment

**Option A: Root-only policy (most restrictive)**
```bash
# 1. Create policy file
sudo tee /etc/wayexpand/policy.toml > /dev/null << 'EOF'
[organization]
safe_mode = true
disable_commands = true
# ... other fields
EOF

# 2. Set restrictive permissions (root-only readable)
sudo chmod 0400 /etc/wayexpand/policy.toml
sudo chown root:root /etc/wayexpand/policy.toml

# 3. Verify
sudo wayexpand doctor --json | jq '.policy.policy'

# 4. Restart running services after policy changes
systemctl --user restart wayexpand-input-method.service
# or: systemctl --user restart wayexpand-evdev.service
```

**Option B: Unprivileged service access (with wayexpand group)**

This allows WayExpand user services to read the policy without needing root.

```bash
# 1. Create wayexpand group if it doesn't exist
sudo groupadd -r wayexpand || true

# 2. Add your user to the wayexpand group
sudo usermod -a -G wayexpand $USER

# 3. Ensure systemd user units run with group membership (see below)

# 4. Create policy file
sudo tee /etc/wayexpand/policy.toml > /dev/null << 'EOF'
[organization]
safe_mode = true
disable_commands = true
# ... other fields
EOF

# 5. Set group-readable permissions
# Note: 0440 allows root and the wayexpand group to read
sudo chmod 0440 /etc/wayexpand/policy.toml
sudo chown root:wayexpand /etc/wayexpand/policy.toml

# 6. Also ensure the directory is owned by root:wayexpand
sudo chown root:wayexpand /etc/wayexpand
sudo chmod 0750 /etc/wayexpand

# 7. Verify (your user must be in wayexpand group; may require logout/login)
wayexpand doctor --json | jq '.policy.policy'

# 8. Restart services
systemctl --user daemon-reload
systemctl --user restart wayexpand-input-method.service
# or: systemctl --user restart wayexpand-evdev.service
```

Note: For Option B to work with systemd user services, your session must start with
group membership (typically requires logging out and back in after `usermod`). Alternatively,
use `newgrp wayexpand` in a new shell to start services with group membership, or configure
the `User=` and `Group=` directives in override files to explicitly set the service context.

Policy is loaded at service startup. Changing `/etc/wayexpand/policy.toml`
does not alter an already-running daemon or IBus engine until the relevant
WayExpand user service or IBus session is restarted. Invalid or insecure policy
continues to fail closed at the next startup/diagnostic check.

## Audit Logging

Policy violations are logged to journald with the configured prefix:

```bash
# View policy violations
journalctl -g "wayexpand-policy" -f

# Example output
wayexpand-policy: command execution is disabled by organization policy
wayexpand-policy: backend 'evdev' is not in allowed list: ["input-method-v2"]
```

## Future: Action Broker

For fine-grained command control (planned for v1.3+), an Action Broker will enable per-action permission control. This feature is tracked in the [PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md).

## See Also

- [FLEET_CONFIG.md](FLEET_CONFIG.md) - Multi-layer configuration
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet deployment
- [OPERATIONS.md](OPERATIONS.md) - Operational deployment guide
