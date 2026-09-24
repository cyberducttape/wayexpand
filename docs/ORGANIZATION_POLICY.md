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
- No group/world writable bits
- Maximum size: 100 KB (DoS prevention)
- Parent directory `/etc/wayexpand/` also validated

## Policy Fields

```toml
[organization]
# Enforcement mode: true = block violations, false = warn only
safe_mode = false

# Disable all command execution (expansions with commands are blocked)
disable_commands = false

# Disable hotkey execution (hotkeys parse but don't run)
disable_hotkeys = false

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
```bash
# 1. Create policy file
sudo tee /etc/wayexpand/policy.toml > /dev/null << 'EOF'
[organization]
safe_mode = true
disable_commands = true
# ... other fields
EOF

# 2. Set strict permissions
sudo chmod 0600 /etc/wayexpand/policy.toml
sudo chown root:root /etc/wayexpand/policy.toml

# 3. Verify
wayexpand doctor --json | jq '.policy.policy'
```

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
