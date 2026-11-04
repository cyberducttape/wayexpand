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
`"execution_sandbox_readiness": "pass"`,
`"input_privacy_readiness": "pass"`,
`"configuration_integrity_readiness": "pass"`, and either
`"action_broker_availability_readiness": "pass"` or `"not_required"` before
treating the host as deployment-ready. Also require
`"target_integrity_readiness": "pass"`; non-atomic compatibility overrides
fail this gate, and uncertified atomic routes remain incomplete. Known unavailable kernel/systemd
evidence currently makes hardened readiness degraded; do not treat such a
host as passing the gate. Overall `readiness` is degraded when the sandbox
passes but the active route lacks sensitive-field detection. The
strict profile fails for evdev even if a route reports that capability. The
independent gate results are also available under `gates`, including whether
each gate is required and which checks contributed to its result. Each gate
reports both policy `readiness` and `observed_readiness`: the former can be
`not_required` when a capability is unused, while the latter always summarizes
the checks that were actually run. For example, an unused but stopped broker
has `readiness: "not_required"`, `required: false`, and
`observed_readiness: "fail"`; this remains visible to inventory automation
without making an installation that has no named actions unhealthy. The execution
sandbox gate includes daemon and, when configured, broker confinement checks;
if configured broker actions need a broker that is not running, that gate also
reports `broker_runtime_security_evidence` as unavailable rather than implying
that the broker's confinement was verified.
the Action Broker availability gate reports service/socket health and mandatory
audit readiness, and is required only when configured named actions need it.
For those actions, `action_broker_mandatory_audit` must report `pass`:
`Run Approved Action` requires a running broker with audit enabled, mandatory,
and healthy. A
broker can therefore be reachable while failing its audit or sandbox checks;
the availability result covers audit, while confinement appears only under
execution sandboxing. Input privacy and configuration integrity are evaluated
separately as well. The top-level `schema` field is currently `10` and is
versioned; automation should consume the stable `gates.<name>.readiness`,
`required`, and `checks` fields and reject unrecognized schema versions. The legacy
`sandbox_readiness` key aliases `execution_sandbox_readiness`. The
security report's `active_route` object records the live source, backend,
atomic replacement capability, replacement assurance, focus assurance, and
compatibility mode. `automatic_replacement_mode` distinguishes an atomic
capability, the default refusal for non-atomic routes, and an experimental
compatibility override. It does not claim desktop certification.
`daemon_runtime.upgrade` reports whether the live daemon is current, needs a
restart, is protocol-incompatible, or cannot be verified, and includes a
recovery action for fleet automation.

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
