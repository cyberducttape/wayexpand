# Fleet Configuration Management

Fleet configuration enables organizations to deploy company-wide snippets across multiple user machines without owning personal configurations.

See:
- **Architecture:** [wayexpand-core fleet module](/crates/core/src/fleet.rs)
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

## Conflict Resolution

When the same trigger appears in multiple layers:
- Organization layer wins (highest priority)
- User layer second
- Packs third (lowest priority)

Duplicate triggers across same layer are rejected with clear error messages.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement and compliance
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible playbooks for fleet deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet modules for fleet management
- [OPERATIONS.md](OPERATIONS.md) - Running and maintaining WayExpand
