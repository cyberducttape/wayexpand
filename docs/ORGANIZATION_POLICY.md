# Organization Policy Reference

**Navigation:** [Home](../README.md) > [System Administration](FOR_SYSADMINS.md) > **Organization Policy**

---

> **Status:** Core feature is implemented in the current 1.2.x release line.

Organization policies enforce administrator-defined constraints on text expansions, protecting sensitive environments and preventing unsafe operations.

## Policy File Location

Policies are loaded from `/etc/wayexpand/policy.toml` (root-owned, strictly validated)
by the daemon, CLI diagnostics, and the IBus service. An invalid or insecure
policy is a startup/diagnostic failure rather than a silent fallback.

## Security Validation

Policy files must meet strict security requirements:
- Regular file (not symlink)
- Root ownership (`uid = 0`)
- **Write-protected:** No group or world writable bits allowed (`mode & 0o022 == 0`)
- Maximum size: 100 KB (DoS prevention)
- Parent directory `/etc/wayexpand/` also validated (root-owned, not group/world-writable)

**Readable permission modes** (all valid, read access does not compromise security):
- `0600` (rw-------): Root read/write only
- `0400` (r--------): Root read-only
- `0440` (r--r-----): Root and group-readable
- `0444` (r--r--r--): Universally readable
- Any mode `& 0o022 == 0` (no write bits for group/world)

The policy file is not secret: its security requirement is **integrity** (write-protected), not
confidentiality. Unprivileged WayExpand user services must be able to read it for enforcement.

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

# Require a backend with a protocol-level atomic replacement transaction
require_atomic_replace = false

# Require the input source to report password/sensitive-field focus
require_sensitive_focus = false

# Maximum replacement text size in bytes (0 = unlimited)
max_replacement_size = 65536

# Allowed output backends (empty = all allowed)
allowed_backends = ["libei", "input-method-v2"]

# Allowed curated packs from ~/.local/share/wayexpand/packs/.
# Empty means unrestricted: all discovered packs are allowed.
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
- `require_atomic_replace` and `require_sensitive_focus` reject startup before
  capture begins when the selected source/backend cannot provide the guarantee
- Blocks: commands, hotkeys, title matching, oversized replacements, disallowed backends
- Can require absolute command paths so `program = "git"` is rejected

**Enforcement is consistent across all policy paths:** All expansion checks
(core engine, daemon, CLI diagnostics) enforce the same restrictions. Violations
are never silent; they are always logged and always block execution when
safe_mode is true.

### Audit Mode (Logging Only)
When `safe_mode = false`:
- Policy violations **allow** expansions to proceed
- Violations logged as **warnings** to journald
- Suitable for testing policies before full enforcement
- Enables discovery of problematic snippets without breaking workflows
- `require_absolute_commands = true` warns about relative program names but permits them

**Logging is consistent across all policy paths:** All expansion checks
log the same violation details to journald with the configured audit_prefix.
Violations never silently fail or cause unexpected behavior changes between
audit and safe modes.

## Examples

### Example 1: Locked-Down Environment
```toml
[organization]
safe_mode = true
disable_commands = true
disable_hotkeys = true
require_absolute_commands = true
disable_title_matching = false
require_atomic_replace = true
require_sensitive_focus = true
max_replacement_size = 1024
allowed_backends = ["input-method-v2"]
# Empty is unrestricted, so list the packs explicitly when restricting use.
allowed_packs = ["approved-pack-1"]
audit_prefix = "corp-policy"
```

### Example 2: Sandboxed Local Commands (Not Networked SRE Actions)

This permits command-backed expansions only inside the daemon's existing
systemd restrictions. It does **not** enable network access, credential access,
home-directory writes, or a supported `kubectl`/cloud/Vault workflow. The broker
is the production boundary for named actions; direct executable commands remain
confined to the daemon's service sandbox.

```toml
[organization]
safe_mode = true
disable_commands = false
disable_hotkeys = false
require_absolute_commands = true
disable_title_matching = false
require_atomic_replace = true
require_sensitive_focus = true
max_replacement_size = 65536
allowed_backends = ["input-method-v2"]
allowed_packs = ["approved-local-tools"]
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
# Empty means all packs are allowed; use an explicit allowlist to restrict them.
allowed_packs = []
audit_prefix = "test-policy"
```

## Deployment

### With Ansible
See [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) for fleet configuration.

### With Puppet
See [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) for fleet configuration.

### Manual Deployment

**Simple approach (recommended):** Root creates the policy file readable by all users.

```bash
# 1. Create the policy directory and file
sudo install -d -o root -g root -m 0755 /etc/wayexpand
sudo tee /etc/wayexpand/policy.toml > /dev/null << 'EOF'
[organization]
safe_mode = true
disable_commands = true
# ... other fields
EOF

# 2. Set permissions: root-owned, world-readable, write-protected
sudo chmod 0444 /etc/wayexpand/policy.toml
sudo chown root:root /etc/wayexpand/policy.toml

# 3. Verify the policy is readable by your user
wayexpand doctor --json | jq '.policy.policy'

# 4. Restart services
systemctl --user restart wayexpand-input-method.service
# or: systemctl --user restart wayexpand-evdev.service
```

**Alternative approach:** Use group-based access for restricted visibility.

If you want the policy visible only to a specific administrator group (replace
`WAYEXPAND_ADMIN_GROUP` with a group that exists on the target distribution):

```bash
# 1. Create the policy directory and file
sudo install -d -o root -g WAYEXPAND_ADMIN_GROUP -m 0750 /etc/wayexpand
sudo tee /etc/wayexpand/policy.toml > /dev/null << 'EOF'
[organization]
safe_mode = true
disable_commands = true
# ... other fields
EOF

# 2. Set group-readable permissions
sudo chmod 0440 /etc/wayexpand/policy.toml
sudo chown root:WAYEXPAND_ADMIN_GROUP /etc/wayexpand/policy.toml

# 3. Add your user to the group
sudo usermod -a -G WAYEXPAND_ADMIN_GROUP $USER

# 4. Start a new login session for group membership to take effect
# (logout/login, or: newgrp WAYEXPAND_ADMIN_GROUP)

# 5. Verify
wayexpand doctor --json | jq '.policy.policy'
```

**Key point:** The policy file does not contain secrets—it contains settings that should be enforced.
The security requirement is **write-protection** (root ownership + no group/world writable bits),
not secrecy. Unprivileged WayExpand services must read it to enforce policy.

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

## Named actions through the Action Broker

For fine-grained command control, use a named Action Broker action such as
`action = "cluster-status"`. The broker owns the executable, fixed arguments,
environment allowlist, working directory, timeout, and output policy. Network
and filesystem isolation still require hardening the broker's service unit.

## See Also

- [FLEET_CONFIG.md](FLEET_CONFIG.md) - Multi-layer configuration
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet deployment
- [OPERATIONS.md](OPERATIONS.md) - Operational deployment guide
