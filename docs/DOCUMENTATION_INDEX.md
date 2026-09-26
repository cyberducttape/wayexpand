# WayExpand Documentation Index

Complete guide to all WayExpand documentation, organized by use case and audience.

## Quick Navigation by Use Case

### I want to install WayExpand
→ Start with [GETTING_STARTED.md](GETTING_STARTED.md)
- Installation instructions for your desktop
- First snippet walkthrough
- Links to advanced guides

### Something isn't working
→ See [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
- Common issues and solutions
- Compositor-specific guidance
- Debugging tips

### I'm switching from Espanso
→ Read [MIGRATION_FROM_ESPANSO.md](MIGRATION_FROM_ESPANSO.md)
- Automatic config import
- Feature comparison
- Side-by-side examples

### I need to manage WayExpand on servers/multiple machines
→ Read [FOR_SYSADMINS.md](FOR_SYSADMINS.md)
- Deployment models
- 30+ production-ready snippets
- Fleet management at scale
- Health checks and monitoring

### I'm deploying to my organization
→ Start with [FLEET_CONFIG.md](FLEET_CONFIG.md)
- Multi-layer configuration
- Policy enforcement
- Then see [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) for policy details
- And [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) or [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) for automation

### I want to handle sensitive data safely
→ Start with [THREAT_MODEL.md](../THREAT_MODEL.md)
- Concise protected / partial / out-of-scope security summary
- Backend privacy tradeoffs

Then see [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md)
- Security best practices
- Pattern recommendations
- Integration with secret stores

### I want to understand the security model
→ Start with [THREAT_MODEL.md](../THREAT_MODEL.md)
- One-page threat table
- Command and backend security boundaries
- Links to detailed security docs

Then see [SECURITY.md](../SECURITY.md)
- Vulnerability reporting
- Detailed daemon, config, evdev, command, and socket model

### I want to handle sensitive data safely in snippets
→ See [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md)
- Security best practices
- Pattern recommendations
- Integration with secret stores

### I want to understand what's supported on my desktop
→ Check [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)
- Feature status per desktop
- Known limitations
- Promotion policy
- Run `wayexpand certify --json` and see [CERTIFICATION.md](CERTIFICATION.md) for evidence requirements

### I'm contributing code
→ Start with [DEVELOPMENT.md](DEVELOPMENT.md)
- Development setup
- Testing requirements
- Code review checklist
- Architecture overview

---

## Complete Documentation Map

### Installation & Getting Started

| Document | Audience | Purpose |
|----------|----------|---------|
| [GETTING_STARTED.md](GETTING_STARTED.md) | New users | Install and create first snippet |
| [MIGRATION_FROM_ESPANSO.md](MIGRATION_FROM_ESPANSO.md) | Espanso users | Switch to WayExpand |
| [UPGRADING.md](UPGRADING.md) | Existing users | Upgrade to new versions |

### Operations & Troubleshooting

| Document | Audience | Purpose |
|----------|----------|---------|
| [OPERATIONS.md](OPERATIONS.md) | All users | Daemon management, logs, config reload |
| [TROUBLESHOOTING.md](TROUBLESHOOTING.md) | Users with issues | Fix common problems |
| [TROUBLESHOOTING_CHECKLIST.md](TROUBLESHOOTING_CHECKLIST.md) | Users with issues | Doctor output → diagnosis → remediation |
| [PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md) | Large-library users | Matcher limits and tuning |
| [FOR_SYSADMINS.md](FOR_SYSADMINS.md) | System admins | Deployment, monitoring, 30+ examples |

### Configuration & Customization

| Document | Audience | Purpose |
|----------|----------|---------|
| [CUSTOMIZATION.md](CUSTOMIZATION.md) | GUI users | Themes, language support, colors |
| [CONFIGURATION_LIMITS.md](CONFIGURATION_LIMITS.md) | All users | Resource and safety boundaries |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Advanced users | Exact CLI/JSON/config contracts |
| [COLOR_PACKS.md](COLOR_PACKS.md) | GUI users | GUI theme options |
| [LANGUAGE_SUPPORT.md](LANGUAGE_SUPPORT.md) | Internationalization | Available languages |
| [RETRO_FONTS.md](RETRO_FONTS.md) | GUI users | Retro font options |
| [UI.md](UI.md) | GUI users | User interface guide |

### Enterprise & Security

| Document | Audience | Purpose |
|----------|----------|---------|
| [THREAT_MODEL.md](../THREAT_MODEL.md) | Security-conscious users | Concise protected/partial/out-of-scope threat summary |
| [SECURITY.md](../SECURITY.md) | Security-conscious users | Vulnerability reporting and detailed security model |
| [FLEET_CONFIG.md](FLEET_CONFIG.md) | Enterprise teams | Multi-layer configuration |
| [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) | Enterprise teams | Policy enforcement and compliance |
| [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md) | Enterprise teams | Handling sensitive data |
| [ACTION_BROKER_DESIGN.md](ACTION_BROKER_DESIGN.md) | SRE/security architects | Planned command/action privilege boundary |
| [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) | DevOps/SRE | Ansible playbooks |
| [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) | DevOps/SRE | Puppet modules |

### Backends & Compatibility

| Document | Audience | Purpose |
|----------|----------|---------|
| [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) | All users | What's tested vs experimental |
| [BACKENDS.md](BACKENDS.md) | Advanced users | Backend architecture |
| [BACKENDS_SENSITIVE_FIELDS.md](BACKENDS_SENSITIVE_FIELDS.md) | Advanced users | Password field protection details |
| [EVDEV_ACCESS_DESIGN.md](EVDEV_ACCESS_DESIGN.md) | Security-conscious administrators | Raw-input permission model and tighter-access investigation |
| [COMPOSITOR_MATRIX.md](COMPOSITOR_MATRIX.md) | System integrators | Desktop/protocol combinations |
| [CERTIFICATION.md](CERTIFICATION.md) | QA and system integrators | Machine-readable certification and compositor test requirements |

### Development & Contribution

| Document | Audience | Purpose |
|----------|----------|---------|
| [DEVELOPMENT.md](DEVELOPMENT.md) | Contributors | Building, testing, contributing |
| [PACKAGING.md](PACKAGING.md) | Package maintainers | Building for distros |
| [RELEASING.md](RELEASING.md) | Maintainers | Release process |
| [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) | Contributors/QA | Testing procedures |
| [GUI_PERFORMANCE.md](GUI_PERFORMANCE.md) | GUI contributors | Performance considerations |

### Current Technical and Release References

| Document | Audience | Purpose |
|----------|----------|---------|
| [IBUS_ENGINE.md](IBUS_ENGINE.md) | IBus users and contributors | IBus engine behavior and protocol boundary |
| [RELEASE_PROMOTION_GUIDE.md](../RELEASE_PROMOTION_GUIDE.md) | Release maintainers | Release and package promotion checks |

### Historical Design and Audit Records

These documents remain available as historical context. Their planning language
and status snapshots are not a substitute for the current contracts above.

| Document | Purpose |
|----------|---------|
| [V13_CRITICAL_REGRESSIONS.md](V13_CRITICAL_REGRESSIONS.md) | Historical v1.3 regression findings and fixes |
| [JOURNEY_TO_V13.md](JOURNEY_TO_V13.md) | Historical v1.2/v1.3 implementation narrative |
| [P1_ARCHITECTURE_ROADMAP.md](P1_ARCHITECTURE_ROADMAP.md) | Historical P1 architecture roadmap |
| [ROADMAP_P2_IMPROVEMENTS.md](../ROADMAP_P2_IMPROVEMENTS.md) | Historical P2 audit follow-up roadmap |

---

## Organization by Topic

### By User Role

#### End Users
- [GETTING_STARTED.md](GETTING_STARTED.md) — installation
- [OPERATIONS.md](OPERATIONS.md) — daemon management
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — fixing issues
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — what works where
- [CUSTOMIZATION.md](CUSTOMIZATION.md) — GUI customization
- [COMPATIBILITY.md](COMPATIBILITY.md) — guarantees

#### System Administrators
- [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — all sysadmin needs (30+ snippets)
- [FLEET_CONFIG.md](FLEET_CONFIG.md) — multi-machine setup
- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) — policy enforcement

#### Enterprise Teams
- [FLEET_CONFIG.md](FLEET_CONFIG.md) — configuration layers
- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) — policy and compliance
- [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md) — secure data handling
- [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) or [PUPPET_INTEGRATION.md](PUPPET_INTEGRATION.md) — deployment

#### Contributors
- [DEVELOPMENT.md](DEVELOPMENT.md) — setup and testing
- [BACKENDS.md](BACKENDS.md) — backend architecture and protocol overview
- [PACKAGING.md](PACKAGING.md) — distro packaging
- [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) — test procedures

### By Feature

#### Installation
- [GETTING_STARTED.md](GETTING_STARTED.md)
- [UPGRADING.md](UPGRADING.md)
- [PACKAGING.md](PACKAGING.md) — distro and aarch64 source builds

#### Configuration
- [GETTING_STARTED.md](GETTING_STARTED.md) — basic
- [COMPATIBILITY.md](COMPATIBILITY.md) — contracts
- [CONFIGURATION_LIMITS.md](CONFIGURATION_LIMITS.md) — boundaries

#### Expansion Matching
- [GETTING_STARTED.md](GETTING_STARTED.md) — first snippet
- [PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md) — matcher limits and large libraries
- [COMPATIBILITY.md](COMPATIBILITY.md) — matching modes

#### Command Execution
- [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — examples
- [DEVELOPMENT.md](DEVELOPMENT.md) — architecture

#### App Filtering
- [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — examples
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — support status
- [BACKENDS.md](BACKENDS.md) — technical details

#### Policy Enforcement
- [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) — complete reference
- [FLEET_CONFIG.md](FLEET_CONFIG.md) — deployment

#### Backend Selection
- [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — what's available
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — if issues
- [BACKENDS.md](BACKENDS.md) — technical details

#### Sensitive Data
- [THREAT_MODEL.md](../THREAT_MODEL.md) — concise security boundary
- [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md) — best practices
- [SECURITY.md](../SECURITY.md) — security model

---

## Common Workflows

### "How do I set up WayExpand?"
1. [GETTING_STARTED.md](GETTING_STARTED.md) — install for your desktop
2. [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — verify support
3. [OPERATIONS.md](OPERATIONS.md) — manage daemon

### "How do I create effective snippets?"
1. [GETTING_STARTED.md](GETTING_STARTED.md) — basic format
2. [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — 30+ examples
3. [COMPATIBILITY.md](COMPATIBILITY.md) — available features

### "How do I deploy to my team?"
1. [FLEET_CONFIG.md](FLEET_CONFIG.md) — understand layers
2. [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md) — set policy
3. [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md) — automate
4. [FOR_SYSADMINS.md](FOR_SYSADMINS.md) — monitor and support

### "Something broke, how do I fix it?"
1. [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — find your issue
2. [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — check if supported
3. [OPERATIONS.md](OPERATIONS.md) — check logs

### "I'm switching from Espanso"
1. [MIGRATION_FROM_ESPANSO.md](MIGRATION_FROM_ESPANSO.md) — overview
2. `wayexpand import espanso ~/.config/espanso/default.yml` — import config
3. [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — if issues

### "I want to contribute code"
1. [DEVELOPMENT.md](DEVELOPMENT.md) — setup
2. [DEVELOPMENT.md](DEVELOPMENT.md#code-review-checklist) — standards
3. [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) — understand status
4. [INTEGRATION_TESTING.md](INTEGRATION_TESTING.md) — test procedures

---

## Finding Answers

### By Problem

**Can't install**
→ [GETTING_STARTED.md](GETTING_STARTED.md)

**Snippets not expanding**
→ [TROUBLESHOOTING.md](TROUBLESHOOTING.md), [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md)

**Daemon not running**
→ [OPERATIONS.md](OPERATIONS.md), [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

**Want to know what's supported**
→ [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md), [BACKENDS.md](BACKENDS.md)

**Password fields not protected**
→ [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md), [BACKENDS_SENSITIVE_FIELDS.md](BACKENDS_SENSITIVE_FIELDS.md)

**Need to handle secrets safely**
→ [THREAT_MODEL.md](../THREAT_MODEL.md), [SECRET_MANAGEMENT.md](SECRET_MANAGEMENT.md)

**Want to deploy to team**
→ [FLEET_CONFIG.md](FLEET_CONFIG.md), [ORGANIZATION_POLICY.md](ORGANIZATION_POLICY.md), [ANSIBLE_INTEGRATION.md](ANSIBLE_INTEGRATION.md)

**Want to contribute**
→ [DEVELOPMENT.md](DEVELOPMENT.md)

---

## Archived Documentation

Historical documentation is available in [archive/](archive/):
- Implementation roadmaps
- Planning documents
- Alternative documentation structures (wiki)
- Historical window tracking guides

For current guidance, see the live documentation above. Archived docs are for reference only.

---

## External Resources

- **GitHub Issues:** https://github.com/cyberducttape/wayexpand/issues
- **Questions and bug reports:** https://github.com/cyberducttape/wayexpand/issues
- **Security:** See [THREAT_MODEL.md](../THREAT_MODEL.md) for the concise
  security boundary and [SECURITY.md](../SECURITY.md) for vulnerability reporting
- **Changelog:** [CHANGELOG.md](../CHANGELOG.md)

---

## Documentation Statistics

- **Total public docs:** 28 files
- **Archived docs:** 11 files
- **Total lines:** ~8,500 lines of documentation
- **Last updated:** 2026-09-23

---

## Navigation Tips

- **Breadcrumbs:** Each major document shows its place in the hierarchy
- **Cross-references:** Documents link to related topics
- **Workflow guides:** See "Common Workflows" section above for guided paths
- **Search:** Use your browser's search (Ctrl+F) to find topics

Welcome! Start with the workflow that matches your needs above. 🎉
