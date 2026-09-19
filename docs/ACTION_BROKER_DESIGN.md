# Action Broker: Separating Capture from Execution

**Status:** Design proposal for v1.3+  
**Audience:** SREs, DevOps teams, enterprise administrators

## Problem

Currently, WayExpand runs keyboard capture and command execution in the same daemon process. This means:

```
User's keyboard
     ↓
wayexpand-daemon (capture + execute)
     ↓
Command output
```

**Issues:**
- All commands run with daemon's permissions (typically user-level)
- No granular per-action permission control
- Secrets must be passed as env vars or embedded in replacement text
- Audit logging can't distinguish command intent from simple policy enforcement

## Solution: Action Broker Architecture

Separate the daemon into two independent services:

```
User's keyboard
     ↓
wayexpand-daemon (capture + match, NO execution)
     ↓
Authenticated IPC (Unix socket)
     ↓
wayexpand-action-broker (execution only)
     ↓
Command output
```

**Benefits:**
- Daemon runs with minimal permissions (only needs input/output)
- Action broker can run with different permissions (e.g., service account)
- Per-action configuration enables fine-grained permission control
- Secrets injected per-action, never stored in config
- Clear audit trail: expansion → action → execution

## Configuration Example

### Before (Current)

```toml
[[expansion]]
trigger = ":kctx"
replacement = ""
command = { program = "/usr/bin/kubectl", args = ["config", "current-context"] }
```

No way to restrict: networking, environment access, timeout, output size.

### After (Proposed)

In expansion (remains in primary config):
```toml
[[expansion]]
trigger = ":kctx"
replacement = ""
action = "kubectl_current_context"
```

In action broker config (`~/.config/wayexpand/actions.toml` or `/etc/wayexpand/actions.d/`):
```toml
[action.kubectl_current_context]
program = "/usr/bin/kubectl"
args = ["config", "current-context"]
timeout_ms = 2000
max_output_bytes = 1024
environment = ["KUBECONFIG"]  # Only these env vars passed through
network = false              # Forbid network access
sandbox = true               # seccomp/pledge/pledge equivalent

# Future: per-action secret management
# secrets = ["KUBECONFIG"]   # Injected at execution time
```

## Deployment Patterns

### Pattern 1: Personal User

```
~/.config/wayexpand/actions.toml
└─ User's personal actions (no secrets)
```

**Permissions:** User-level only.

### Pattern 2: Team/Organization

```
/etc/wayexpand/actions.d/approved-actions.toml
└─ Pre-approved actions (organization secrets)

~/.config/wayexpand/actions.toml
└─ User overrides (if allowed by policy)
```

**Permissions:**
- Daemon: User-level (no exec)
- Action broker: Service account with org secrets

### Pattern 3: SRE with Infrastructure Actions

```
/etc/wayexpand/actions.d/
├─ kubectl.toml        (runs as sre-kubectl user)
├─ aws.toml            (runs as sre-aws user)
└─ terraform.toml      (runs as sre-terraform user)
```

**Permissions:**
- Daemon: Restricted user (only capture/match)
- kubectl action broker: sre-kubectl (has ~/.kube/config)
- aws action broker: sre-aws (has ~/.aws/credentials)
- terraform action broker: sre-terraform (has terraform state)

## Implementation Roadmap

### Phase 1: IPC Plumbing (v1.3)
- Define Unix socket protocol between daemon and broker
- Implement authenticated IPC (SO_PEERCRED verification)
- Add `wayexpand-action-broker` service
- Maintain backward compatibility (single-process mode as default)

### Phase 2: Action Configuration (v1.3)
- Extend expansion config with `action` field
- Implement action broker config loading
- Add per-action timeout, output limit, env filtering
- CLI: `wayexpand action list`, `wayexpand action test <name>`

### Phase 3: Permission Enforcement (v1.4)
- Sandbox support (seccomp/pledge)
- Per-action secret injection (from /etc or secret manager)
- Policy integration: allowed_actions, required_action_approval
- Audit logging: who triggered what action when

### Phase 4: SRE Features (v1.4+)
- Integration with systemd service accounts
- Multi-broker deployment (different services on different machines)
- Centralized action library (git repo or central server)
- Action approval workflows (optional per-action confirmation)

## Comparison: Action Broker vs Current Model

| Aspect | Current | Action Broker |
|--------|---------|---------------|
| Permission isolation | No | Yes (per-action service account) |
| Granular permission control | No | Yes |
| Secret management | Env vars (daemon scope) | Per-action injection |
| Audit trail | Expansion matches | Expansion → action → execution |
| Sandbox support | No | Yes (seccomp/pledge) |
| Multi-user actions | No | Yes |
| Enterprise readiness | Partial | Full |

## Why This Differentiates WayExpand

**Current alternatives (Espanso, AutoHotkey, etc.):**
- Execute everything in-process with user permissions
- No permission separation
- All secrets in one config file
- Limited to single-user environments

**WayExpand with Action Broker:**
- SRE-focused permission model
- Secrets isolated per-action
- Multi-user deployment patterns
- Enterprise-grade audit trails
- Enables infrastructure automation at the desktop

This is what makes WayExpand suitable for **infrastructure teams** (DevOps, SREs, platform engineers) rather than just individual productivity users.

## See Also

- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) - Current policy model
- [PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md) - Broader development roadmap
- [FOR_SYSADMINS.md](FOR_SYSADMINS.md) - Deployment guidance
