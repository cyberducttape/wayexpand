# Enterprise Roadmap: SRE/DevOps/Sysadmin Focus

**Vision:** WayExpand as professional-grade text expansion for operations teams. Deployment-friendly, auditable, secret-aware, observable.

**Key principle:** Fleet configuration and operational observability before performance optimization.

The standard fleet merge is active in the daemon when no explicit config path
is supplied. The primary user config remains the base layer; organization,
user `snippets.d`, and pack directories are then merged in that order. Use
`wayexpand fleet status` to inspect the discovered layers.

## Phase 1: Fleet Configuration & Policy (v1.3-1.4)

### 1.1 Multi-Layer Configuration Merging

**Why:** Sysadmins must deploy company snippets without owning users' personal configs.

**Design:**
```
/etc/wayexpand/snippets.d/          # Organization policy (root-owned)
~/.config/wayexpand/snippets.d/    # User personal snippets
~/.local/share/wayexpand/packs/    # Optional curated packs
```

**Features:**
- Deterministic merge order (primary config → organization → user → packs)
- `validate --merge-preview` to dry-run combined config
- `fmt`, `lint`, `diff` subcommands for config management
- Source provenance tracking (which file defines each snippet)
- `wayexpand validate` warns on conflicts/precedence

**Ansible/Puppet integration:**
- Deploy to `/etc/wayexpand/snippets.d/` via config management
- User personal config stays at `~/.config/wayexpand/expansions.toml`
- No file conflicts, auditable changes

**Impact:** Organizations can deploy snippets centrally without user friction.

### 1.2 Organization Safe Mode

**Why:** Root-managed policies for security/compliance.

**Config:**
```toml
[organization]
safe_mode = true
disable_commands = true
disable_hotkeys = false
disable_title_matching = false
max_replacement_size = 65536
allowed_packs = ["sre-core", "kubectl"]
allowed_backends = ["input-method", "libei"]
```

**Enforcement:**
- Violation warnings in `doctor` and `validate`
- Command-backed expansions refuse to run if `disable_commands = true`
- Hotkeys ignored if `disable_hotkeys = true`
- Daemon refuses unsupported backends

**Audit trail:**
- Config validation logs which policies are active
- Each violation is logged with policy name and reason

**Impact:** Enterprises can deploy with confidence in a locked-down environment.

---

## Phase 2: Backend Capability Contracts (v1.4)

### 2.1 Explicit Backend Feature Matrix

**Why:** DevOps needs to know deployment constraints *before* going to production.

**Each backend declares:**
```rust
pub struct BackendCapabilities {
    pub name: &'static str,
    pub sensitive_focus_aware: bool,           // Pause in password fields?
    pub cursor_position_tracking: bool,        // Know where to insert?
    pub max_atomic_insertion: usize,          // Largest single insert?
    pub multiline_support: bool,               // Newlines in expansions?
    pub pass_through_keys: bool,               // Arrows, Escape, etc.?
    pub persistent_authorization: bool,       // One-time permission?
    pub keyboard_layout_fidelity: bool,        // Unicode support?
    pub latency_percentile_ms: (u32, u32),    // p50, p99?
    pub known_limitations: Vec<&'static str>, // Explicit warnings
}
```

### 2.2 Deployment Validation

**`wayexpand validate --backend=evdev --strict`:**
```
✓ evdev: global keyboard capture enabled
✓ libei: portal persistent, one-time consent
⚠ evdev: NO sensitive field detection (password expansion will NOT pause)
⚠ evdev: input group grants global session keyboard visibility
✓ libei: multiline support, full Unicode
✗ STRICT: title matching not supported on evdev (no window title available)
```

**Decision support:**
- Compare backends side-by-side: `wayexpand compare-backends`
- Verify chosen backend meets organizational requirements before deployment

**Impact:** DevOps makes informed decisions; no surprises in production.

---

## Phase 3: Secret Management (v1.4-1.5)

### 3.1 Secret Provider Integration

**Why:** Encourage secrets outside TOML config files.

**Supported providers:**
- Secret Service (systemd/libsecret)
- `pass` (standard Unix password manager)
- 1Password, Bitwarden (via CLI)
- HashiCorp Vault

**Config:**
```toml
[[expansion]]
trigger = ":dbpass"
replacement = "{{ secret('database/prod', 'password') }}"
secret_provider = "vault"
vault_addr = "https://vault.company.example.com"
vault_token_path = "~/.config/wayexpand/vault-token"  # mode 0600
```

**Safety guarantees:**
- Secrets NEVER logged, even in debug mode
- Secrets NEVER appear in `--preview` or GUI preview
- Secrets NOT included in `support-bundle`
- Secret interpolation happens at expansion time, never stored

**Vault integration:**
- Token stored at `~/.config/wayexpand/vault-token` (mode 0600)
- Daemon rotates tokens if Vault indicates expiry
- Failed secret retrieval: expansion fails closed (no blank text)

**Impact:** Eliminates API tokens and passwords in TOML files; enterprise-grade secret handling.

---

## Phase 4: Operational Observability (v1.5-1.6)

### 4.1 Structured Status & Diagnostics

**`wayexpand status --json`:**
```json
{
  "daemon": {
    "running": true,
    "pid": 12345,
    "uptime_seconds": 86400,
    "last_healthy": "2026-09-20T15:30:45Z",
    "config_last_reloaded": "2026-09-20T08:00:00Z",
    "config_reload_failures": 0
  },
  "backends": {
    "input_source": "input-method",
    "injector": "libei",
    "input_method": { "connected": true, "protocol": "zwp_input_method_v2" },
    "libei": { "connected": true, "persistent_auth": true }
  },
  "metrics": {
    "expansions_today": 1247,
    "failures_today": 3,
    "avg_latency_ms": 12.4,
    "buffer_usage_chars": 256,
    "config_reload_count": 5
  }
}
```

**`wayexpand doctor --json`:**
```json
{
  "healthy": true,
  "config": {
    "path": "/home/user/.config/wayexpand/expansions.toml",
    "valid": true,
    "snippets_count": 342,
    "organization_policy": "/etc/wayexpand/snippets.d/",
    "policy_snippets_count": 89,
    "safe_mode_active": false
  },
  "backends": [
    {
      "name": "input-method-v2",
      "available": true,
      "protocol_detected": true,
      "sensitive_focus": true,
      "cursor_tracking": true,
      "reason": "KDE Plasma 6.6 detected"
    }
  ]
}
```

### 4.2 Journald Integration

**Structured fields for log parsing:**
```
MESSAGE_ID=abcd1234
PRIORITY=6
SYSLOG_IDENTIFIER=wayexpand
WAYEXPAND_COMPONENT=daemon
WAYEXPAND_EVENT=config_reload
WAYEXPAND_RESULT=success
WAYEXPAND_SNIPPET_COUNT=350
WAYEXPAND_DURATION_MS=125
```

**No keystroke logging, ever.**

### 4.3 Metrics & Monitoring

**Counters (prometheus-friendly):**
- `wayexpand_expansions_total` (by status: success, failed, blocked)
- `wayexpand_config_reloads_total`
- `wayexpand_backend_reconnects_total`
- `wayexpand_expansion_latency_ms` (histogram: p50, p95, p99)
- `wayexpand_buffer_usage_chars` (gauge)

**No PII, no keystroke content, no trigger names, ever.**

**Impact:** SREs can monitor fleet deployments; alerting on failures; observability without privacy concerns.

---

## Phase 5: Support Bundle & Diagnostics (v1.6)

### 5.1 Redacted Support Archive

**`wayexpand support-bundle`:**

Produces `wayexpand-support-bundle-2026-09-20.tar.gz`:
```
metadata/
  version                    # v1.5.3
  git-sha                    # 0a1b2c3d...
  build-date                 # 2026-09-15T12:00Z
  distro                     # Ubuntu 22.04
  kernel                     # 6.5.0-31-generic
  wayland-version            # 1.24
  compositor                 # Sway 1.8

backends/
  capabilities.json          # Feature matrix for active backends
  doctor-output.json         # Full doctor --json

systemd/
  status                     # systemctl --user status wayexpand
  hardening-analysis         # security analysis of unit

security/
  config-policy             # /etc/wayexpand/snippets.d/ structure (no values)
  vault-status              # Connected to Vault? Token valid?

diagnostics/
  recent-logs               # Last 1000 journald lines (sanitized)
  expansions-histogram      # Expansion count by category
  error-summary             # Failures grouped by reason

package/
  source                    # APT/RPM package source
  installed-version         # dpkg -l | grep wayexpand
```

**Redaction rules:**
- Config file names but NOT contents
- Snippet trigger names but NOT replacements
- Backend connection status but NO credentials
- Error messages but NO user data
- Timestamps but NO keystroke timing correlations

**SRE benefit:** Dramatically better bug reports; no accidental secrets; compliance-friendly sharing.

---

## Phase 6: Forms & Parameterized Snippets (v1.7)

### 6.1 Snippet Templates with Variables

**Why:** Cluster → namespace → resource pattern common in operations.

**Config:**
```toml
[[expansion]]
trigger = ":pod"
form = "pod_lookup"

[expansions.form.pod_lookup]
type = "interactive"
steps = [
  { name = "cluster", options = ["us-west", "us-east", "eu"] },
  { name = "namespace", depends_on = "cluster" },
  { name = "pod_name", searchable = true },
]
template = "kubectl -c {cluster} get pod {pod_name} -n {namespace}"
```

**UX:**
- Type `:pod`, daemon shows palette with cluster selector
- Each selection populates downstream fields
- Live search for pod names
- Escape to cancel, Enter to expand

**Impact:** Complex multi-step operations become single trigger.

### 6.2 Curated SRE Snippet Packs

**Why:** Baseline expectations for mature tooling; shared across teams.

**Official packs (versioned, inspectable):**
- `sre-core`: Common patterns (kubectl, systemctl, git, curl, ssh)
- `kubernetes-v1.24+`: kubectl snippets for current K8s versions
- `journalctl-recipes`: Common log queries
- `incident-response`: Postmortem templates, escalation contacts
- `infrastructure-as-code`: Terraform, Ansible snippets
- `networking`: netstat, tcpdump, traceroute, dig patterns

**Deployment:**
```toml
[organization]
enabled_packs = [
  { name = "sre-core", version = "2.1.0" },
  { name = "kubernetes-v1.24+", version = "3.0.0" },
]
```

**Trust model:**
- Packs are versioned and immutable
- SHA256 hash verified on load
- Source provenance tracked (Git SHA, repository URL)
- Eventually: signed by project maintainers

**Impact:** Organizations save weeks of snippet authoring; consistency across teams.

---

## Phase 7: Professional Linux Packaging (v1.8)

### 7.1 Signed APT/RPM Repositories

**Why:** Enterprise standard for software distribution.

- GPG-signed Releases, Packages indices
- APT: `deb [signed-by=/etc/apt/keyrings/wayexpand.gpg] https://ppa.wayexpand.org focal main`
- RPM: repo with `.gpg_verify=1`
- Checksum verification of all artifacts
- SBOM (Software Bill of Materials) for compliance
- Provenance metadata (build date, maintainer, Git SHA)

### 7.2 Multi-Architecture, Properly Tested

- x86_64: Primary, CI-tested on every commit
- aarch64: Genuinely tested (not cross-compiled), CI-gated releases
- Armv7: If demand exists, properly gated

### 7.3 System Integration

- Man pages: `man wayexpand`, `man wayexpand-daemon`
- Shell completions: bash, zsh, fish
- Systemd unit files shipped in package
- AppStream metadata for GUI discovery
- Desktop entry for graphical launcher
- Package upgrade tests: upgrading from v1.0 → v1.8 works cleanly

### 7.4 Support for Distribution Packaging

- Arch AUR (community-maintained)
- Fedora Copr (official)
- Debian/Ubuntu PPA (official)
- openSUSE Build Service (community)
- NixOS (community-maintained)

**Impact:** Sysadmins can deploy via standard package managers; auditable, reproducible installations.

---

## Phase 8: Performance Optimization (v1.9+)

**Only after correctness + operability are locked down.**

- Compact trie/automaton for matcher (current: hash-based, fast enough)
- Pre-normalized app IDs for `app_filter`
- O(1)-style hotkey lookup (current: O(n) scan)
- Daemon startup latency profiling
- Matcher benchmarks for 10K+ snippet configs

**Why late:** Operations teams rarely complain about millisecond expansions; they complain about deployment friction, observability, and management overhead.

---

## Implementation Priority

**Tier 1 (v1.3 - 6 weeks):** Fleet configuration, safe mode
**Tier 2 (v1.4 - 6 weeks):** Backend capabilities, secret providers, observability
**Tier 3 (v1.5 - 4 weeks):** Support bundle, refined metrics
**Tier 4 (v1.6+ - ongoing):** Forms, curated packs, packaging polish

**Why this order:**
1. Deployment and policy (blocks all enterprise adoption without this)
2. Secrets and observability (security and operational confidence)
3. Support/debugging (makes production issues tractable)
4. Advanced UX (quality-of-life for heavy users)
5. Packaging polish (professional presentation)
6. Performance (nice to have after everything else works)

---

## Success Metrics for Enterprise Adoption

- [ ] One company (5+ users) publicly deploying via fleet config
- [ ] Support bundle solves 80% of reported issues on first try
- [ ] Vault/1Password integration working and documented
- [ ] Signed APT/RPM repos accepting PRs from package maintainers
- [ ] 50+ curated SRE snippets in official packs
- [ ] Zero security incidents from secrets leaking via logs/preview

---

## Differentiators vs. Consumer Text Expanders

| Feature | WayExpand | Espanso | AutoKey |
|---------|-----------|---------|---------|
| Fleet config | ✅ Multi-layer | ❌ Single user | ❌ Single user |
| Backend capabilities matrix | ✅ Explicit | ❌ Implicit | ❌ Unknown |
| Organization policy | ✅ Safe mode | ❌ No policy | ❌ No policy |
| Secret manager integration | ✅ Vault/1Password | ❌ Config only | ❌ Hardcoded |
| Structured observability | ✅ Journald/JSON | ❌ Ad hoc | ❌ Limited |
| Support bundle | ✅ Redacted archive | ❌ None | ❌ None |
| Curated packs | ✅ Versioned SRE packs | ✅ Community packages | ❌ None |
| Linux packaging | ✅ Signed repos | ✅ Community only | ❌ None |

**The story:** Enterprise-grade operations software, not a consumer utility.
