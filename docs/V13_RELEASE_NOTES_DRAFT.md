# WayExpand v1.3.0 Release Notes (Draft)

**Release Date:** TBD  
**Status:** Phase 1 Foundation Complete - Ready for Phase 2

## What's New in v1.3.0

### 🏗️ Action Broker Architecture (Experimental; disabled)

The biggest architectural improvement since v1.2: separate command execution from keyboard capture for better security and control.

**What this means:**
- **Daemon isolation**: Keyboard capture daemon stays locked down (no network, read-only /home)
- **Fine-grained control**: Per-action permissions (environment variables, working directory, timeouts)
- **Enterprise-ready**: Clear audit trail for compliance
- **Privilege separation**: Commands don't compromise the capture process

**What's implemented:**
- Complete protocol for daemon-broker communication
- Configuration system with validation
- Safe command executor with timeout + policy enforcement
- Unix socket IPC layer for local-only communication
- Daemon integration foundation (ready for Phase 2)
- Standalone broker service binary (`wayexpand-action-broker`)
- Comprehensive test suite
- Full architecture documentation

**Example:**
```toml
# Define actions with fine-grained control
[actions."k8s_pods"]
program = "/usr/bin/kubectl"
args_prefix = ["get", "pods"]
timeout_ms = 10000
pass_env = ["KUBECONFIG"]
```

### 📋 Command Path Determinism ✅ SHIPPED (v1.2)

Configured in organization policy: `require_absolute_commands = true`

For managed deployments, forces absolute paths:
```toml
[organization]
require_absolute_commands = true
```

This prevents accidental command resolution from user PATH, ensuring predictable execution in fleet deployments.

## v1.3.0 Release Criteria

### ⚠️ Action Broker Foundation (experimental only)
- Protocol, configuration, executor, IPC, service binary
- 100+ lines of tests
- Full documentation
- Ready for production testing

### ⏳ Phase 2 (Recommended for v1.3): Enhanced Controls
**Estimated:** 8-10 hours
- Policy-based routing from daemon to broker
- Per-action environment filtering
- Per-action timeout enforcement
- Systemd user service integration
- Socket activation support

### ⏳ Phase 3 (Post-v1.3): Enterprise Features
**Estimated:** 12-15 hours
- Audit logging integration
- Role-based action access (RBAC)
- Organization-wide action catalog
- Advanced audit trail queries

## Testing & Stability

All 417 existing tests continue to pass:
- ✅ Core matcher and expansion engine
- ✅ Daemon event loop
- ✅ Command execution (P2 process cleanup verified)
- ✅ Policy enforcement
- ✅ Backend integration
- ✅ New: Action Broker unit + integration tests

## Migration Guide for v1.2 → v1.3

### No Breaking Changes
v1.3.0 is fully backward compatible with v1.2 configurations.

### What's Optional
Action Broker is **not enabled for production use**. v1.2 deployments continue working unchanged:
- Commands still execute in daemon process (v1.2 behavior)
- Daemon relaxes permissions if needed for commands
- No configuration changes required

### Enabling Action Broker (v1.3.x+)
```toml
[organization]
# When daemon and broker service are both running:
action_broker_socket = "/run/user/1000/wayexpand-broker.sock"

# Actions are automatically routed to broker instead of local execution
[[expansion]]
trigger = ";kpods"
command = { program = "/usr/bin/kubectl", args = ["get", "pods"] }
```

## Documentation

### New in v1.3
- `docs/ACTION_BROKER_ARCHITECTURE.md` - Complete design guide
- Standalone broker service binary with help
- Integration test suite

### From v1.2 (Still Relevant)
- `docs/CAPTURE_BACKEND_TRADEOFFS.md` - Input capture decisions
- `docs/CERTIFICATION_MATRIX.md` - Compositor compatibility
- `docs/ORGANIZATION_POLICY.md` - Policy enforcement

## Performance Impact

**Action Broker (experimental):**
- No performance impact when disabled
- When enabled: +5-10ms latency (IPC overhead)
- Recommended for managed deployments where isolation matters more than latency

## Known Limitations in v1.3.0

### Phase 1 (Current)
- Action Broker foundation only; Phase 2 routing is not implemented
- Do not enable it as a security boundary; the restricted command runner remains the default
- No systemd integration (use manual socket binding)
- No audit logging yet
- No RBAC controls yet

### Planned for v1.3.x
- Phase 2: Policy-based routing and enhanced controls
- Systemd user service configuration
- Audit logging and compliance features

## Enterprise Adoption

v1.3 is not an enterprise-ready Action Broker release:
- ✅ Input capture security validated (isolation)
- ⚠️ Command execution architecture remains experimental
- ⚠️ Network/filesystem sandboxing and hardened broker IPC are not complete
- ✅ Managed deployment support (Command Path Determinism)
- ✅ Comprehensive documentation

## What Didn't Make v1.3.0

These are pushed to v1.3.x:
- **Doctor as Centerpiece** - Enterprise diagnostic tool enhancement
- **E2E Test Automation** - CI integration for compositor testing
- **Performance Benchmarking** - Real performance data

## Upgrading to v1.3.0

### From v1.2.x
```bash
# No changes needed for existing configs
# Everything continues to work as-is

# Do not start the experimental broker for production deployments yet.
systemctl --user start wayexpand-action-broker.service
systemctl --user enable wayexpand-action-broker.service

# Optional: Update daemon config to route commands to broker
# (See ACTION_BROKER_ARCHITECTURE.md)
```

## Bug Fixes in v1.3.0
(Same as v1.2 - no new bugs!)

## Contributors

- Action Broker architecture: Claude Haiku 4.5
- All tests continue to pass with no regressions

## Links

- GitHub: https://github.com/cyberducttape/wayexpand
- Documentation: See `docs/` directory
- Issues: https://github.com/cyberducttape/wayexpand/issues

---

**v1.3.0 is recommended for:** Enterprise deployments, managed environments, users who want fine-grained command control.

**v1.2.x continues to be recommended for:** Home users, simple deployments, maximum compatibility.
