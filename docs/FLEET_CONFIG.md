# Fleet Configuration Management

**Navigation:** [Home](../README.md) > [System Administration](FOR_SYSADMINS.md) > **Fleet Configuration**

---

Fleet configuration enables organizations to deploy company-wide snippets across multiple user machines without owning personal configurations.

See:
- **Architecture:** [`wayexpand-core` fleet module](../crates/core/src/fleet.rs)
- **Enterprise deployment:** [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md), [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md)
- **Policy enforcement:** [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md)
- **Configuration layers:**
  1. `/etc/wayexpand/snippets.d/` - Organization layer (root-owned)
  2. `~/.config/wayexpand/snippets.d/` - User layer (personal snippets)
  3. `~/.local/share/wayexpand/packs/` - Curated packs

## Quick Start

### Organization Layer (root-owned)
```sh
sudo mkdir -p /etc/wayexpand/snippets.d
sudo tee /etc/wayexpand/snippets.d/org-snippets.toml > /dev/null << 'EOF'
[[expansion]]
trigger = ";sig"
replacement = "Best regards,\nYour Organization"

[[expansion]]
trigger = ";legal"
replacement = "[Legal footer here]"
EOF
sudo chmod 644 /etc/wayexpand/snippets.d/org-snippets.toml
```

### User Layer
```sh
mkdir -p ~/.config/wayexpand/snippets.d
tee ~/.config/wayexpand/snippets.d/personal.toml > /dev/null << 'EOF'
[[expansion]]
trigger = ";bye"
replacement = "Goodbye!"
EOF
```

### Using Curated Packs
```sh
mkdir -p ~/.local/share/wayexpand/packs/my-pack
# Add TOML files to my-pack/
```

## Policy Control

Organizations can restrict which packs are allowed:

```toml
[organization]
allowed_packs = ["approved-pack-1", "approved-pack-2"]
```

## Layer Precedence and Conflict Resolution

**Layer Loading Order:** Organization → User → Packs → Base config (primary config appended last, lowest priority)

**Duplicate/Conflict Behavior:**

| Object | Duplicate behavior | Notes |
|--------|-------------------|-------|
| Expansion trigger | Hard error, rejected | Fail-closed: prevents accidental overwrites. Use distinct trigger names. |
| Hotkey chord | Hard error, rejected | Fail-closed: prevents key binding conflicts. |
| Settings (max_replacement_size, etc.) | Last layer wins | Pack settings override user, which override organization. Within a layer, last file wins. |
| Organization policy | Organization layer wins | If fleet organization policy exists, it replaces base config policy entirely. |
| Curated packs | Filtered by policy | Organization policy `allowed_packs` restricts which packs are active. |
| Base config | Appended last (lowest priority) | Fleet layers are merged first, then base config expansions/hotkeys are appended. Base settings only override if fleet has no settings. |

**Example precedence:**
- If organization defines `max_replacement_size = 1024` and pack defines `max_replacement_size = 2048`, pack value wins (2048).
- If organization defines `;sig` trigger and user also defines `;sig`, deployment fails with duplicate-trigger error.
- Base config's existing snippets are appended to fleet snippets (no override, no error).

**For Infrastructure:** Ensure distinct trigger/hotkey names across organizational, user, and pack layers. Use policy enforcement (`safe_mode = true`) to catch duplicate-trigger errors during validation before deployment.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement and compliance
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible playbooks for fleet deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet modules for fleet management
- [OPERATIONS.md](OPERATIONS.md) - Running and maintaining WayExpand
