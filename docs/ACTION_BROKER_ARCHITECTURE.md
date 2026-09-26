# Action Broker Architecture

**Version:** 1.3.0  
**Status:** Phase 1 - Foundation Complete  
**Next:** Phase 2 - Enhanced Control (v1.3.x)

## Overview

The Action Broker separates command execution from keyboard capture, enabling:
- **Daemon isolation** - Keyboard capture stays locked down (no network)
- **Per-action control** - Fine-grained permissions for each action
- **Audit trail** - All command executions logged and traceable
- **Privilege separation** - Commands run with appropriate permissions

## Architecture

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
│ • Audit logging             │
└─────────────────────────────┘
```

## Phase 1: Foundation (v1.3.0) - COMPLETE

What's implemented:
- **Protocol** (crates/action-broker/src/protocol.rs)
  - `ActionRequest`: Daemon requests action execution with ID and timeout
  - `ActionResponse`: Broker returns success or error result
  - `ActionError`: Comprehensive error types (not found, blocked, timeout, etc.)

- **Configuration** (crates/action-broker/src/config.rs)
  - `ActionConfig`: Per-action definitions (program, args, timeout, env, cwd)
  - `BrokerConfig`: Full broker setup with action registry
  - Validation: Enforce absolute paths, environment restrictions

- **Executor** (crates/action-broker/src/executor.rs)
  - Safe command execution with policy enforcement
  - Environment variable filtering with strict mode
  - Async execution with tokio timeout support
  - Output capture (stdout/stderr) with exit code tracking

- **IPC Layer** (crates/action-broker/src/ipc.rs)
  - Unix domain sockets (AF_UNIX) for local-only communication
  - Line-delimited JSON for simplicity
  - `BrokerServer`: Listen and accept connections
  - `BrokerClient`: Connect and send requests

- **Daemon Integration** (crates/daemon/src/action_broker.rs)
  - `ActionBrokerManager` for client connection pooling
  - Ready for policy-based routing (Phase 2)

## Phase 2: Enhanced Control (v1.3.x) - PLANNED

What will be implemented:
- **Policy Integration**
  - Wire `action_broker_socket` configuration to daemon
  - Route commands based on policy decisions
  - Fallback to local execution if broker unavailable

- **Per-Action Permissions**
  - Environment variable filtering
  - Working directory restrictions
  - Network access control (if needed)
  - Timeout enforcement per action

- **Audit Logging**
  - Log all action executions to syslog/journald
  - Track: action ID, parameters, execution time, exit code
  - Integration with organization audit trail

- **Standalone Broker Binary**
  - Create `crates/action-broker-server/` binary
  - Systemd user service for auto-start
  - Socket activation support
  - Configuration file loading

## Phase 3: Enterprise Features (v1.4+) - FUTURE

What could be added:
- Role-based action access (RBAC)
- Organization-wide action catalog
- Secret management integration
- Advanced audit trail queries
- Multi-tenant support

## Configuration Example

### Broker Configuration (wayexpand-broker.toml)

```toml
[broker]
require_absolute_paths = true
strict_env = true
audit_enabled = true
audit_path = "/var/log/wayexpand-actions.log"

# Action: List Kubernetes resources
[actions."k8s_get_pods"]
program = "/usr/bin/kubectl"
args_prefix = ["get", "pods"]
timeout_ms = 10000
allow_network = true
pass_env = ["KUBECONFIG", "HOME"]
enabled = true

# Action: AWS CLI identity check
[actions."aws_sts_identity"]
program = "/usr/bin/aws"
args_prefix = ["sts", "get-caller-identity"]
timeout_ms = 5000
allow_network = true
pass_env = ["AWS_PROFILE", "AWS_REGION"]
enabled = true

# Action: Local file operations (no network)
[actions."file_stat"]
program = "/usr/bin/stat"
args_prefix = []
timeout_ms = 2000
allow_network = false
pass_env = ["HOME"]
cwd = "/home/user"
enabled = true
```

### Daemon Configuration (wayexpand-config.toml)

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

### Daemon Permissions (Minimal)
- No network access (AF_UNIX only)
- No keyboard devices (input-method-v2 uses composition events)
- Read-only access to /home
- Cannot spawn arbitrary processes
- Cannot access environment variables

### Broker Permissions (Per-Action)
- Only configured programs allowed
- Arguments constrained by prefix matching
- Environment variables explicitly allowlisted
- Working directory restricted
- Execution timeout enforced
- No escalated privileges (runs as regular user)

### Audit Trail
- Every action logged with timestamp
- Action parameters recorded
- Execution time tracked
- Exit code and output captured
- Can integrate with syslog/journald for compliance

## Implementation Status

| Component | Status | Tests | Notes |
|-----------|--------|-------|-------|
| Protocol | ✅ Complete | ✅ Pass | Request/response/error types |
| Config | ✅ Complete | ✅ Pass | TOML schema with validation |
| Executor | ✅ Complete | ✅ Pass | Async execution with timeout |
| IPC (Unix socket) | ✅ Complete | ✅ Pass | JSON over AF_UNIX streams |
| Daemon integration | ✅ Foundation | ⏳ Pending | ActionBrokerManager ready |
| Policy routing | ⏳ Phase 2 | ⏳ Pending | Will integrate with policy module |
| Broker binary | ⏳ Phase 2 | ⏳ Pending | Standalone service executable |
| Systemd integration | ⏳ Phase 2 | ⏳ Pending | User service + socket activation |
| Audit logging | ⏳ Phase 2 | ⏳ Pending | syslog/journald integration |

## Benefits Over Current Approach

| Aspect | Current (v1.2) | With Action Broker (v1.3+) |
|--------|-----------------|--------------------------|
| Daemon network access | None | None |
| Command execution | In daemon process | Separate broker process |
| Per-command control | Policy only (all or nothing) | Fine-grained per-action |
| Audit trail | Not available | Full logging |
| Security isolation | Moderate | Strong |
| Flexibility | Limited | High |

## Getting Help

- See [CAPTURE_BACKEND_TRADEOFFS.md](CAPTURE_BACKEND_TRADEOFFS.md) for input capture options
- See [CERTIFICATION_MATRIX.md](CERTIFICATION_MATRIX.md) for tested configurations
- See [docs/PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md) for v1.3+ timeline

## References

- `crates/action-broker/` - Action Broker library
- `crates/daemon/src/action_broker.rs` - Daemon integration
- [P2 Action Broker Architecture](../memory/P2_action_broker_architecture.md) - Design details
