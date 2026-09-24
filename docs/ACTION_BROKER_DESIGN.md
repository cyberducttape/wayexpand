# Action Broker: Separating Capture from Execution

**Status:** P1 security architecture gate; not implemented
**Audience:** SREs, DevOps teams, enterprise administrators

This document defines the boundary required before WayExpand can claim to
support infrastructure actions. The current daemon does **not** implement an
Action Broker, and its direct command feature remains limited by the daemon's
systemd sandbox. Do not weaken that sandbox to make `kubectl`, `aws`, `vault`,
`ssh`, or `terraform` work.

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

### After (Target design; not yet supported)

In expansion (remains in primary config):
```toml
[[expansion]]
trigger = ":kctx"
replacement = ""
action = "kubectl_current_context"
```

The `action` field above is deliberately not accepted by the current 1.1
configuration schema. It is the target broker-facing schema, not a workaround
that users can enable today.

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

### Phase 0: Security and protocol gate (before implementation)
- Keep direct daemon commands available only for sandbox-compatible, trusted
  local actions.
- Define the broker protocol and threat model below.
- Add integration tests proving that the capture daemon cannot request an
  arbitrary executable, pass arbitrary environment variables, or bypass the
  broker's allowlist.
- Document that SRE workflows are unsupported until this gate is complete.

### Phase 1: IPC Plumbing (v1.3)
- Define the Unix socket protocol between daemon and broker.
- Implement authenticated IPC (`SO_PEERCRED` verification on Linux).
- Add the `wayexpand-action-broker` service with a separate systemd unit.
- Maintain backward compatibility: direct command mode remains the default for
  existing local snippets, while broker actions are explicit opt-in.

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

## Broker protocol and trust boundary

The broker is a privilege and capability boundary, not merely a second command
worker. The daemon sends an action identifier; it must not send a program path,
shell text, unrestricted arguments, or an environment map.

The initial Linux transport should be a user-owned `AF_UNIX` stream socket in
`$XDG_RUNTIME_DIR/wayexpand/action-broker.sock`, mode `0600`, with the broker
verifying the connecting process credentials (`SO_PEERCRED`) before reading a
request. The broker must also validate its own socket and configuration
parents, matching the existing control-socket ownership rules. A future
organization deployment may use a separately managed broker user and an
explicitly trusted group, but that must be a deliberate policy choice rather
than an ambient filesystem permission.

Requests should use bounded length-prefixed JSON for the first implementation:

```json
{
  "protocol": 1,
  "request_id": "uuid-or-random-opaque-id",
  "action": "kubectl_current_context"
}
```

The broker response must be similarly bounded and distinguish these outcomes:
`accepted`, `completed`, `denied`, `timed_out`, `failed`, and `unavailable`.
Responses may contain bounded UTF-8 stdout for text-producing actions, but must
never include secrets in ordinary audit logs. The broker owns the action's
absolute executable path, fixed arguments, explicit environment allowlist,
working directory, network policy, timeout, output limit, and audit metadata.

The broker must reject unknown fields, unknown action names, oversized frames,
duplicate or malformed request IDs, shell metacharacter interpretation, and
requests that attempt to override action policy. The daemon must fail closed
when the broker is unavailable; it must never fall back to executing the
requested SRE command locally.

Each action execution needs an audit record containing the authenticated
caller, action name, request ID, start/end result, timeout/failure category,
and output byte count. It must not contain typed triggers, replacement text,
secret values, or arbitrary command arguments.

### Acceptance criteria

The broker is not ready for production until all of the following are tested:

1. The hardened capture daemon has no network or home-directory access even
   when an action request is triggered.
2. A fake broker can prove framing, credential checks, timeout handling, and
   bounded output without starting an arbitrary process.
3. An action configuration owned by another user, writable by group/other, or
   containing an unresolved executable path is rejected before startup.
4. The daemon cannot use a broker denial or disconnect as permission to run the
   command itself.
5. The broker's systemd unit has an independently reviewed hardening profile
   and service-account/secret ownership model.
6. Action execution is contained at the service or cgroup boundary so a
   malicious action cannot escape cleanup by forking, calling `setsid()`, or
   retaining inherited output descriptors after the direct child exits.

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
