# Action Broker Architecture

**Version:** 1.2.0
**Status:** Integrated named-action execution; network/filesystem isolation remains a deployment responsibility
**Next:** Managed service packaging and per-action OS sandbox profiles

## Overview

The Action Broker separates named command execution from keyboard capture. A
snippet may reference `action = "cluster-status"`; the daemon sends only that
name over a protected Unix socket. The broker's executable, fixed arguments,
working directory, timeout, output capture, and environment allowlist remain
server-side policy. The broker is not itself an OS sandbox: network and
filesystem isolation still require service-level hardening.

The standalone binary is part of normal workspace builds and release packages. Set
`WAYEXPAND_ACTION_BROKER_SOCKET` for the daemon (or use the default socket under
`$XDG_RUNTIME_DIR`) and run the broker with a mode-0600 configuration.

The design aims to provide:
- **Daemon isolation** - Keyboard capture stays locked down (no network)
- **Per-action control** - Fine-grained permissions for each action
- **Audit trail** - Optional privacy-preserving JSONL execution audit sink
- **Privilege separation** - Commands run with appropriate permissions

## Runtime architecture

The daemon's bounded command worker routes named actions through this socket;
ordinary `program =` commands retain the existing restricted direct runner.

An expansion references only the broker action name:

```toml
[[expansion]]
trigger = ":cluster"
replacement = ""

[expansion.command]
action = "cluster-status"
timeout_ms = 3000
```

The corresponding broker policy fixes what may run:

```toml
[actions."cluster-status"]
program = "/usr/bin/kubectl"
args = ["cluster-info"]
timeout_ms = 3000
server_env = ["KUBECONFIG"]
cwd = "/home/stephan"
```

`server_env` values are read from the broker's environment and cannot be
overridden by an IPC client. Use `client_forward_env` only for values that a
same-UID client is explicitly allowed to provide. Loader and interpreter
variables such as `LD_PRELOAD`, `LD_LIBRARY_PATH`, `PYTHONPATH`, `PERL5LIB`,
`RUBYOPT`, `BASH_ENV`, `ENV`, and `PATH` are rejected unless an action opts into
`allow_dangerous_env = true`. The broker authenticates the peer UID; it is not
a sandbox against compromised software running as the same desktop user.

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

## Current implementation (integrated, operator-configured)

What's implemented:
- **Protocol** (crates/action-broker/src/protocol.rs)
  - `ActionRequest`: Daemon requests action execution with ID and timeout
  - `ActionResponse`: Broker returns success or error result
  - `ActionError`: Comprehensive error types (not found, blocked, timeout, etc.)

- **Configuration** (crates/action-broker/src/config.rs)
  - `ActionConfig`: Per-action definitions (program, args, timeout, env, cwd)
  - `BrokerConfig`: Full broker setup with action registry
  - Validation: Enforce absolute paths and private working directories, verify
    regular executable ownership and permissions plus trusted canonical parents,
    canonicalize paths, and apply environment restrictions at broker startup;
    package upgrades require a broker restart to revalidate targets

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
  - Bounded client connections (64) so idle same-user clients cannot exhaust
    the broker's blocking request workers

- **Standalone service binary** (`wayexpand-action-broker`)
  - Loads a validated broker configuration
  - Binds a protected Unix socket
  - Accepts bounded, authenticated requests and executes configured actions
  - Is a tested workspace binary shipped by release installers and distro packages
  - Has an operator-managed `wayexpand-action-broker.service` unit; it is not
    enabled automatically because the broker configuration is deployment-specific

- **Execution audit sink**
  - Enable with `[broker] audit_path = "/private/path/action-audit.jsonl"`
  - Writes bounded JSONL events with request ID, action ID, caller PID and
    executable when available, policy SHA-256, timing, exit/timeout status, and
    output byte count
  - Never records arguments, environment values, stdout, or stderr
  - The sink is mode `0600`, synchronously appended, and capped at 16 MiB;
    write failures are reported to the service journal while execution proceeds

To build the standalone broker from a checkout:

```sh
  cargo build --locked -p action-broker \
  --bin wayexpand-action-broker
```

The packaged user service uses `~/.config/wayexpand/broker.toml` and
`$XDG_RUNTIME_DIR/wayexpand-broker.sock`; enable it after creating that policy.

## Next: Enhanced Control (v1.3.x) - PLANNED

What will be implemented:
- **Policy Integration**
  - Add richer policy controls around the already-integrated action route
  - Fail closed when the broker is unavailable; never fall back to local daemon execution

- **Per-Action Permissions**
  - Environment variable filtering
  - Working directory restrictions
  - Network isolation through a concrete service/container sandbox (not a TOML boolean)
  - Timeout enforcement per action

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
server_env = ["KUBECONFIG", "HOME"]
enabled = true

# Action: AWS CLI identity check
[actions."aws_sts_identity"]
program = "/usr/bin/aws"
args = ["sts", "get-caller-identity"]
timeout_ms = 5000
client_forward_env = ["AWS_PROFILE", "AWS_REGION"]
enabled = true

# Action: Local file operations (no network)
[actions."file_stat"]
program = "/usr/bin/stat"
args = []
timeout_ms = 2000
server_env = ["HOME"]
cwd = "/home/user"
enabled = true
```

### Daemon routing configuration

```toml
# Optional override for the daemon's broker socket.
# The packaged service uses the default under $XDG_RUNTIME_DIR.
# Set WAYEXPAND_ACTION_BROKER_SOCKET in the daemon service environment if needed.

[[expansion]]
trigger = ";kpods"
replacement = ""

[expansion.command]
action = "k8s_get_pods"

[[expansion]]
trigger = ";aws-id"
replacement = ""

[expansion.command]
action = "aws_sts_identity"
```

## Security Model

### Current daemon execution model
- The daemon's mature direct-command path can spawn configured command processes.
- Named `action` commands are sent to the standalone broker over its protected
  Unix socket; they are not executed in the daemon process.
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
- Optional: set `[broker] audit_path` to enable the broker's JSONL execution
  sink.
- Events contain timing, action identity, request ID, peer metadata, policy
  hash, exit/timeout status, and output size, but not arguments, environment
  values, stdout, or stderr.
- The sink is capped at 16 MiB and uses `sync_data`; persistence failures are
  journaled and do not block action execution.

## Implementation Status

| Component | Status | Tests | Notes |
|-----------|--------|-------|-------|
| Protocol | ✅ Complete | ✅ Pass | Request/response/error types |
| Config | ✅ Complete | ✅ Pass | TOML schema with validation |
| Executor | ✅ Complete | ✅ Pass | Async execution with timeout |
| IPC (Unix socket) | ✅ Complete | ✅ Pass | JSON over AF_UNIX streams |
| Daemon integration | ✅ Named actions | ✅ Pass | Core routes actions to the standalone broker |
| Policy routing | ✅ Action policy | ✅ Pass | Broker validates the configured action catalog |
| Broker binary | ✅ Integrated standalone binary | ✅ Workspace and package builds | `wayexpand-action-broker`; operator-configured service |
| Systemd integration | ✅ User service | ✅ Verified | Operator enables the packaged broker unit |
| Audit logging | ✅ Optional JSONL sink | ✅ Pass | Privacy-preserving, bounded, mode `0600` |

## Benefits Over Current Approach

| Aspect | Current (v1.2) | With Action Broker (v1.3+) |
|--------|-----------------|--------------------------|
| Daemon network access | None | None |
| Command execution | Direct `program` commands in daemon process | Named `action` commands in separate broker process |
| Per-command control | Policy only (all or nothing) | Fine-grained per-action |
| Audit trail | Not available | Optional privacy-preserving execution events |
| Security isolation | Moderate | Strong process/policy boundary; same-UID trust remains |
| Flexibility | Limited | High |

## Getting Help

- See [CAPTURE_BACKEND_TRADEOFFS.md](CAPTURE_BACKEND_TRADEOFFS.md) for input capture options
- See [CERTIFICATION_MATRIX.md](CERTIFICATION_MATRIX.md) for tested configurations
- See [docs/PROFESSIONAL_ROADMAP.md](../PROFESSIONAL_ROADMAP.md) for v1.3+ timeline

## References

- `crates/action-broker/` - Action Broker library
- `crates/action-broker/Cargo.toml` - Standalone broker binary manifest
- [Action Broker security architecture](ACTION_BROKER_ARCHITECTURE.md) - Current design and security status
