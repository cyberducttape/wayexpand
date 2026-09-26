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
umask 077
mkdir -p ~/.config/wayexpand/snippets.d
tee ~/.config/wayexpand/snippets.d/personal.toml > /dev/null << 'EOF'
[[expansion]]
trigger = ";bye"
replacement = "Goodbye!"
EOF
```

### Using Curated Packs
```sh
umask 077
mkdir -p ~/.local/share/wayexpand/packs/my-pack
# Add TOML files to my-pack/
```

User-owned TOML files must be mode `0600`; the `umask` above keeps personal
snippets and pack files private when they are created with a text editor or
shell redirection. Organization-layer files are root-owned and may use the
documented `0644` mode.

## Policy Control

Fleet layer files contain snippets, hotkeys, and settings only. Security policy
is loaded from the root-owned `/etc/wayexpand/policy.toml`; an `[organization]`
table in a fleet layer is rejected. In daemon fleet mode, that root policy's
`allowed_packs` list is enforced when `safe_mode = true`; with safe mode off,
disallowed packs are retained and reported as audit-only policy violations.

## Layer Precedence and Conflict Resolution

**Layer Loading Order:** Base config → Organization → User → Packs (base config is prepended and has the lowest priority)

**Duplicate/Conflict Behavior:**

| Object | Duplicate behavior | Notes |
|--------|-------------------|-------|
| Expansion trigger | Hard error, rejected | Fail-closed: prevents accidental overwrites. Use distinct trigger names. |
| Hotkey chord | Hard error, rejected | Fail-closed: prevents key binding conflicts. |
| Settings (`max_buffer_chars`, `undo_chord`, `font_scale`, `libei_token_persistence`) | Last layer wins | Pack settings override user, which override organization. Within a layer, last file wins; disallowed pack settings are filtered before precedence is resolved. |
| Organization policy | Root policy only | `/etc/wayexpand/policy.toml` is the administrator security-policy source. |
| Curated packs | Filtered by root policy in safe mode | `allowed_packs` restricts which packs are active in daemon fleet mode only when enforcement is enabled. |
| Base config | Prepended first (lowest priority) | Base config expansions/hotkeys precede fleet layers. Base settings only override if fleet has no settings. |

**Example precedence:**
- If organization defines `;sig` trigger and user also defines `;sig`, deployment fails with duplicate-trigger error.
- Base config's existing snippets precede fleet snippets, then the merged configuration is validated. Duplicate triggers or hotkeys across the base and fleet layers cause deployment to fail; there is no override behavior.

**For Infrastructure:** Ensure distinct trigger/hotkey names across organizational, user, and pack layers. Validate `/etc/wayexpand/policy.toml` separately before deployment.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement and compliance
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible playbooks for fleet deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet modules for fleet management
- [OPERATIONS.md](OPERATIONS.md) - Running and maintaining WayExpand
