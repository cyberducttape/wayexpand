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

Fleet layer files contain snippets, hotkeys, and settings only. Security policy
is loaded from the root-owned `/etc/wayexpand/policy.toml`; an `[organization]`
table in a fleet layer is rejected. In daemon fleet mode, that root policy's
`allowed_packs` list filters curated packs.

## Layer Precedence and Conflict Resolution

**Layer Loading Order:** Organization → User → Packs → Base config (primary config appended last, lowest priority)

**Duplicate/Conflict Behavior:**

| Object | Duplicate behavior | Notes |
|--------|-------------------|-------|
| Expansion trigger | Hard error, rejected | Fail-closed: prevents accidental overwrites. Use distinct trigger names. |
| Hotkey chord | Hard error, rejected | Fail-closed: prevents key binding conflicts. |
| Settings (`max_buffer_chars`, `undo_chord`, `font_scale`, `libei_token_persistence`) | Last layer wins | Pack settings override user, which override organization. Within a layer, last file wins; disallowed pack settings are filtered before precedence is resolved. |
| Organization policy | Root policy only | `/etc/wayexpand/policy.toml` is the administrator security-policy source. |
| Curated packs | Filtered by root policy | `allowed_packs` restricts which packs are active in daemon fleet mode. |
| Base config | Appended last (lowest priority) | Fleet layers are merged first, then base config expansions/hotkeys are appended. Base settings only override if fleet has no settings. |

**Example precedence:**
- If organization defines `;sig` trigger and user also defines `;sig`, deployment fails with duplicate-trigger error.
- Base config's existing snippets are appended to fleet snippets, then the merged configuration is validated. Duplicate triggers or hotkeys across the base and fleet layers cause deployment to fail; there is no override behavior.

**For Infrastructure:** Ensure distinct trigger/hotkey names across organizational, user, and pack layers. Validate `/etc/wayexpand/policy.toml` separately before deployment.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement and compliance
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible playbooks for fleet deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet modules for fleet management
- [OPERATIONS.md](OPERATIONS.md) - Running and maintaining WayExpand
