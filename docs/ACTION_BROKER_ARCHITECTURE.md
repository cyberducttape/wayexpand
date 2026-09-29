# Action Broker Architecture

**Version:** 1.2.0
**Status:** Experimental foundation; disabled by default and not a security boundary
**Next:** Policy routing, service integration, and external sandbox integration before enablement

## Overview

The Action Broker is an experimental design for separating command execution
from keyboard capture. It is not enabled by default and must not be treated as
a sandbox: network and filesystem isolation are deployment responsibilities
until a concrete service/container policy is integrated. The mature restricted
command runner remains the safer default.

The design aims to provide:
- **Daemon isolation** - Keyboard capture stays locked down (no network)
- **Per-action control** - Fine-grained permissions for each action
- **Audit trail** - Planned; no execution audit sink is currently implemented
- **Privilege separation** - Commands run with appropriate permissions

## Target architecture (not current routing)

The following diagram is the intended Phase 2 topology. In the current v1.2
implementation, the daemon does not route expansion commands through this
socket: its mature command path still executes configured commands in the
daemon process under the daemon's policy and service sandbox. The standalone
broker binary can be run manually, but no daemon policy or service lifecycle
starts it automatically.

```
┌─────────────────────────────┐
│ Keyboard Capture Daemon     │
│ • No network (AF_UNIX only) │
│ • Read-only /home           │
│ • Input + Matching + Inject │
└────────────────┬────────────┘
                 │
         [Authenticated IPC]
         Unix Socket (AF_UNIX)
                 │
                 ▼
┌─────────────────────────────┐
│ Action Broker Service       │
│ • User privileges           │
│ • Per-action control        │
│ • Environment filtering     │
│ • Timeout enforcement       │
│ • Authenticated requests    │
└─────────────────────────────┘
```

## Current implementation (experimental, not production-ready)

What's implemented:
- **Protocol** (crates/action-broker/src/protocol.rs)
  - `ActionRequest`: Daemon requests action execution with ID and timeout
  - `ActionResponse`: Broker returns success or error result
  - `ActionError`: Comprehensive error types (not found, blocked, timeout, etc.)

- **Configuration** (crates/action-broker/src/config.rs)
  - `ActionConfig`: Per-action definitions (program, args, timeout, env, cwd)
  - `BrokerConfig`: Full broker setup with action registry
  - Validation: Enforce absolute paths and private working directories, verify
    regular executable ownership and permissions, canonicalize paths, and apply
    environment restrictions at broker startup; package upgrades require a broker
    restart to revalidate targets

- **Executor** (crates/action-broker/src/executor.rs)
  - Safe command execution with policy enforcement
  - Environment variable filtering with strict mode
  - Deadline-aware process-group termination and reaping
  - Bounded output capture (stdout/stderr) with exit code tracking
  - Bounded concurrent execution (16 actions per broker process)

- **IPC Layer** (crates/action-broker/src/ipc.rs)
  - Unix domain sockets (AF_UNIX) for local-only communication
  - Line-delimited JSON for simplicity
  - `BrokerServer`: Listen and accept connections
  - `BrokerClient`: Connect and send requests

- **Daemon-side broker helper** (crates/daemon/src/action_broker.rs)
  - `ActionBrokerManager` is a tested, opt-in helper for future routing
  - It opens a fresh client connection for each request; it is not a pool
  - It is currently unused by the daemon command route
- **Standalone service binary** (`wayexpand-action-broker`)
  - Loads a validated broker configuration
  - Binds a protected Unix socket
  - Accepts bounded, authenticated requests and executes configured actions
  - Is a tested workspace binary; current distribution installers do not ship
    it because no managed service or daemon routing is enabled yet

## Phase 2: Enhanced Control (v1.3.x) - PLANNED

What will be implemented:
- **Policy Integration**
  - Wire `action_broker_socket` configuration to daemon
  - Route commands based on policy decisions
  - Fallback to local execution if broker unavailable

- **Per-Action Permissions**
  - Environment variable filtering
  - Working directory restrictions
  - Network isolation through a concrete service/container sandbox (not a TOML boolean)
  - Timeout enforcement per action

- **Audit Logging**
  - Log all action executions to syslog/journald
  - Track: action ID, parameters, execution time, exit code
  - Integration with organization audit trail

- **Standalone Broker Deployment**
  - Add a systemd user service and optional socket activation
  - Document operator-managed startup and lifecycle
  - Integrate daemon policy routing and action-name configuration

## Phase 3: Enterprise Features (v1.4+) - FUTURE

What could be added:
- Role-based action access (RBAC)
- Organization-wide action catalog
- Secret management integration
- Advanced audit trail queries
- Multi-tenant support

## Configuration examples (current broker schema and target routing)

### Current standalone broker configuration (wayexpand-broker.toml)

```toml
[broker]
# Defaults to true; set false only for an intentional PATH-based deployment.
require_absolute_paths = true
strict_env = true

# Action: List Kubernetes resources
[actions."k8s_get_pods"]
program = "/usr/bin/kubectl"
args = ["get", "pods"]
timeout_ms = 10000
pass_env = ["KUBECONFIG", "HOME"]
enabled = true

# Action: AWS CLI identity check
[actions."aws_sts_identity"]
program = "/usr/bin/aws"
args = ["sts", "get-caller-identity"]
timeout_ms = 5000
pass_env = ["AWS_PROFILE", "AWS_REGION"]
enabled = true

# Action: Local file operations (no network)
[actions."file_stat"]
program = "/usr/bin/stat"
args = []
timeout_ms = 2000
pass_env = ["HOME"]
cwd = "/home/user"
enabled = true
```

### Target daemon routing configuration (not active in v1.2)

```toml
[organization]
# Phase 1: Optional (no routing yet)
# Phase 2: Will enable automatic routing
action_broker_socket = "/run/user/1000/wayexpand-broker.sock"

[[expansion]]
trigger = ";kpods"
replacement = ""
# Phase 2: Will route to broker
# action = "k8s_get_pods"

[[expansion]]
trigger = ";aws-id"
replacement = ""
# Phase 2: Will route to broker
# action = "aws_sts_identity"
```

## Security Model

### Current daemon execution model
- The daemon's mature command path can spawn configured command processes.
- Expansion and organization policy are checked before dispatch.
- The daemon service sandbox supplies additional restrictions such as no
  network access and read-only home/system protection.
- This is not equivalent to the separate-process isolation described below;
  the daemon is not currently a broker-only command router.

### Target broker permissions (when routing is enabled)
- The keyboard daemon would retain a minimal AF_UNIX-only permission set and
  would not spawn action commands itself.
- Only configured programs allowed
- Action arguments are fixed by the validated action definition; any future
  request-supplied arguments must use a separately constrained allowlisted
  model rather than being inferred from the current `args` array
- Environment variables explicitly allowlisted
- Working directory restricted
- Execution timeout enforced
- No escalated privileges (runs as regular user)

### Audit Trail
- Not implemented in the current broker binary.
- The configuration schema intentionally has no audit switch or path; adding
  such a setting before a real, tested sink exists would create a false
  compliance signal.
- Audit logging remains a planned Phase 2 capability and must define its
  privacy, rotation, failure, and integrity semantics before enablement.

## Implementation Status

| Component | Status | Tests | Notes |
|-----------|--------|-------|-------|
| Protocol | ✅ Complete | ✅ Pass | Request/response/error types |
| Config | ✅ Complete | ✅ Pass | TOML schema with validation |
| Executor | ✅ Complete | ✅ Pass | Async execution with timeout |
| IPC (Unix socket) | ✅ Complete | ✅ Pass | JSON over AF_UNIX streams |
| Daemon integration | ⚠️ Helper only | ✅ Unit/integration tests | `ActionBrokerManager` exists but is not wired into command routing; reconnects per request |
| Policy routing | ⏳ Phase 2 | ⏳ Pending | Will integrate with policy module |
| Broker binary | ✅ Source implementation | ✅ Pass | `wayexpand-action-broker`; current distribution installers omit it until service deployment and routing are defined |
| Systemd integration | ⏳ Phase 2 | ⏳ Pending | User service + socket activation |
| Audit logging | ⏳ Phase 2 | ⏳ Pending | syslog/journald integration |

## Benefits Over Current Approach

| Aspect | Current (v1.2) | With Action Broker (v1.3+) |
|--------|-----------------|--------------------------|
| Daemon network access | None | None |
| Command execution | In daemon process under policy/sandbox | Separate broker process after Phase 2 routing |
| Per-command control | Policy only (all or nothing) | Fine-grained per-action |
| Audit trail | Not available | Planned; not implemented |
| Security isolation | Moderate | Strong |
| Flexibility | Limited | High |

## Getting Help

- See [CAPTURE_BACKEND_TRADEOFFS.md](CAPTURE_BACKEND_TRADEOFFS.md) for input capture options
- See [CERTIFICATION_MATRIX.md](CERTIFICATION_MATRIX.md) for tested configurations
- See [docs/PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md) for v1.3+ timeline

## References

- `crates/action-broker/` - Action Broker library
- `crates/daemon/src/action_broker.rs` - Daemon integration
- [Action Broker security architecture](ACTION_BROKER_ARCHITECTURE.md) - Current design and security status
