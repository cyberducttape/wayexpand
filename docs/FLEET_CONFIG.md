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
  2. `snippets.d/` next to the configuration file - User layer (personal
     snippets; `~/.config/wayexpand/snippets.d/` by default, or under
     `$XDG_CONFIG_HOME/wayexpand/` when that is set)
  3. `${XDG_DATA_HOME:-~/.local/share}/wayexpand/packs/` - Curated packs

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
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/wayexpand/packs/my-pack"
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

Example of a restrictive pilot policy (the route must provide every requested
capability or WayExpand refuses the deployment):

```toml
[organization]
safe_mode = true
disable_commands = true
disable_hotkeys = true
require_signed_packs = true
require_sensitive_focus = true
minimum_replacement_guarantee = "verified"
allowed_backends = ["ibus", "input-method-v2"]
```

The `require_sensitive_focus` setting is the enforced control for capture
privacy: it refuses evdev because evdev cannot identify password or other
sensitive fields. `allowed_backends` limits insertion backends and does not by
itself identify or prohibit an evdev capture source. After starting the
managed user service, require
`wayexpand doctor --security --profile hardened --json` to report
`"sandbox_readiness": "pass"` and `"input_privacy_readiness": "pass"` before
treating the host as deployment-ready. Overall `readiness` is degraded when
the sandbox passes but the active route lacks sensitive-field detection. The
strict profile fails for evdev even if a route reports that capability. The
security report's `active_route` object records the live source, backend,
atomic replacement capability, replacement assurance, focus assurance, and
compatibility mode. `automatic_replacement_mode` distinguishes an atomic
capability, the default refusal for non-atomic routes, and an experimental
compatibility override. It does not claim desktop certification.

Inspect the policy's enforcement view with `wayexpand policy inspect`. CI
should use `wayexpand policy check --file ./policy.toml --json`; its exit code
is 0 when the policy is enforceable, 1 when restrictions are only audit-mode
or known unsupported controls are requested, 2 for invalid command usage, and
3 for an invalid policy file. The JSON report includes each control's
configured/effective value and the reason a setting is audit-only or
unsupported. `--allow-audit-only` is an explicit inspection override and does
not make those restrictions enforceable. Candidate files are syntax and
semantics checked but are not treated as trusted deployed policy. `doctor
--json` and `fleet status --json` include the same policy view. Session-dependent
requirements still need runtime route verification.

## Layer Precedence and Conflict Resolution

**Snippet Loading Order:** Base config → Organization → User → Packs. This order resolves snippets with the same stable ID and controls duplicate-trigger checks; duplicates with different IDs remain errors.

**Settings Precedence:** Settings merge field by field. Curated packs supply defaults, organization layer settings supply administrator defaults, and the primary config plus user layer supply personal preferences. Within each category, later files override earlier files. An explicitly configured user value wins even when it equals WayExpand's built-in default. The root-owned `/etc/wayexpand/policy.toml` remains the source of enforced security restrictions and takes effect independently of these preferences.

Run `wayexpand fleet status` to see the effective settings and their source. `wayexpand fleet status --json` reports the same mapping as `effective_settings`.

**Duplicate/Conflict Behavior:**

| Object | Duplicate behavior | Notes |
|--------|-------------------|-------|
| Expansion trigger | Hard error, rejected | Fail-closed: prevents accidental overwrites. Use distinct trigger names. |
| Hotkey chord | Hard error, rejected | Fail-closed: prevents key binding conflicts. |
| Settings | Field by field | User preferences override organization defaults, which override pack recommendations. Later files win within a category. Enforced restrictions come from root policy. |
| Organization policy | Root policy only | `/etc/wayexpand/policy.toml` is the administrator security-policy source. |
| Curated packs | Filtered by root policy in safe mode | `allowed_packs` restricts which packs are active in daemon fleet mode only when enforcement is enabled. |
| Base config | User preference for settings; first for snippets/hotkeys | Base config expansions/hotkeys precede fleet layers. Explicit settings in the base config override organization and pack defaults. |

**Example precedence:**
- If organization defines `;sig` trigger and user also defines `;sig`, deployment fails with duplicate-trigger error.
- Base config's existing snippets precede fleet snippets, then the merged configuration is validated. Duplicate triggers or hotkeys across the base and fleet layers cause deployment to fail; there is no override behavior.

**For Infrastructure:** Ensure distinct trigger/hotkey names across organizational, user, and pack layers. Validate `/etc/wayexpand/policy.toml` separately before deployment.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Policy enforcement and compliance
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) - Ansible playbooks for fleet deployment
- [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) - Puppet modules for fleet management
- [OPERATIONS.md](OPERATIONS.md) - Running and maintaining WayExpand
