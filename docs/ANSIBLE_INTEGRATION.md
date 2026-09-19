# Ansible Integration for WayExpand Fleet Configuration

Deploy company-wide WayExpand snippets across your infrastructure using Ansible, without owning users' personal configurations.

## Architecture

WayExpand loads snippets from three layers in order:

1. **Organization layer:** `/etc/wayexpand/snippets.d/` (root-owned, managed by Ansible)
2. **User layer:** `~/.config/wayexpand/snippets.d/` (user personal, not touched by Ansible)
3. **Pack layer:** `~/.local/share/wayexpand/packs/` (optional curated packs)

Each layer is a directory of `.toml` files. Within a layer, files are loaded alphabetically. Duplicate triggers are rejected with source provenance.

## Ansible Role Example

### Directory Structure

```
wayexpand-snippets/
├── tasks/
│   └── main.yml
├── files/
│   ├── 01-sre-core.toml
│   ├── 02-kubernetes.toml
│   └── 03-incident-response.toml
├── templates/
│   └── organization.toml.j2
├── vars/
│   └── main.yml
└── meta/
    └── main.yml
```

### tasks/main.yml

```yaml
---
- name: Ensure wayexpand config directory exists
  file:
    path: /etc/wayexpand/snippets.d
    state: directory
    owner: root
    group: root
    mode: '0755'

- name: Deploy organization snippet pack
  copy:
    src: "{{ item }}"
    dest: /etc/wayexpand/snippets.d/
    owner: root
    group: root
    mode: '0644'
  loop:
    - 01-sre-core.toml
    - 02-kubernetes.toml
    - 03-incident-response.toml
  notify: reload wayexpand daemon

- name: Deploy templated organization config
  template:
    src: organization.toml.j2
    dest: /etc/wayexpand/snippets.d/organization.toml
    owner: root
    group: root
    mode: '0644'
  notify: reload wayexpand daemon

- name: Verify fleet configuration
  command: wayexpand validate --merge-preview
  register: validation_result
  failed_when: validation_result.rc != 0
  changed_when: false
```

### templates/organization.toml.j2

```toml
# Organization-wide WayExpand snippets
# Managed by Ansible - do not edit manually
# Generated for: {{ ansible_hostname }}

[[expansion]]
trigger = ":companyname"
replacement = "{{ company_name }}"
description = "Insert company name"
category = "company"

[[expansion]]
trigger = ":orgid"
replacement = "{{ organization_id }}"
description = "Insert organization ID"
category = "company"

[[expansion]]
trigger = ":oncall"
replacement = "{{ oncall_contact }}"
description = "Current on-call contact"
category = "operations"
```

### vars/main.yml

```yaml
---
company_name: "ACME Inc"
organization_id: "acme-prod-2026"
oncall_contact: "oncall@acme.example.com"
```

### handlers/main.yml

```yaml
---
- name: reload wayexpand daemon
  systemd:
    name: wayexpand-input-method.service
    state: restarted
    scope: user
    daemon_reload: yes
  become: yes
  become_user: "{{ item }}"
  loop: "{{ query('getent', 'passwd') | map(attribute=0) | list }}"
  when:
    - item not in ['root', 'sync', 'shutdown', 'halt', 'nobody']
    - item.startswith('user') or item in ['alice', 'bob', 'eve']
```

## Usage

### Deploy to hosts

```bash
ansible-playbook -i inventory deploy-wayexpand.yml
```

### Playbook example

```yaml
---
- hosts: sre-team
  roles:
    - wayexpand-snippets
  vars:
    company_name: "ACME Inc"
    organization_id: "acme-prod-2026"
```

### With safe mode

To deploy with organization policy enforcement:

```yaml
---
- hosts: all
  roles:
    - wayexpand-snippets
  tasks:
    - name: Create organizational policy file
      copy:
        content: |
          [organization]
          safe_mode = true
          disable_hotkeys = false
          disable_title_matching = false
          max_replacement_size = 65536
          allowed_backends = ["input-method", "libei"]
        dest: /etc/wayexpand/policy.toml
        owner: root
        group: root
        mode: '0644'
```

## Example Snippet Files

### 01-sre-core.toml

```toml
# Core SRE/DevOps snippets

[[expansion]]
trigger = ":logme"
replacement = "journalctl --user -u wayexpand -n 50 -f"
description = "Tail recent WayExpand logs"
category = "logging"

[[expansion]]
trigger = ":restart-daemon"
description = "Restart WayExpand daemon"
category = "operations"

[expansion.command]
program = "systemctl"
args = ["--user", "restart", "wayexpand-input-method.service"]
timeout_ms = 5000

[[expansion]]
trigger = ":test-snippet"
replacement = "wayexpand validate --merge-preview"
description = "Test fleet snippet merge"
category = "admin"
```

### 02-kubernetes.toml

```toml
# Kubernetes/kubectl snippets

[[expansion]]
trigger = ":kgp"
replacement = "kubectl get pods"
description = "Get pods in current namespace"
category = "kubernetes"

[[expansion]]
trigger = ":kdesc"
replacement = "kubectl describe"
description = "Describe Kubernetes resource"
category = "kubernetes"

[[expansion]]
trigger = ":kctx"
replacement = "kubectl config current-context"
description = "Show current kubectl context"
category = "kubernetes"
```

### 03-incident-response.toml

```toml
# Incident response templates

[[expansion]]
trigger = ":inc"
replacement = """## Incident: 
**Status:** Investigating
**Duration:** 
**Impact:** 
**Owner:** 
**Next Steps:**"""
description = "Incident response template"
category = "incidents"
match_mode = "immediate"

[[expansion]]
trigger = ":postmortem"
replacement = """## Postmortem
**What happened:**

**Why did it happen:**

**Impact:**

**What we changed:**

**Action items:**
- [ ] Item 1"""
description = "Postmortem template"
category = "incidents"
```

## Verification

### Check fleet merge locally

```bash
# View what will be deployed
ansible-playbook --check deploy-wayexpand.yml

# Test on single host
ansible-playbook -i 'target.example.com,' deploy-wayexpand.yml
```

### Verify on target

```bash
# SSH to target
ssh user@target.example.com

# Show organization snippets
cat /etc/wayexpand/snippets.d/*.toml

# Test merge (if wayexpand installed)
wayexpand validate --merge-preview --json | jq '.stats'

# Show provenance
journalctl --user -u wayexpand | grep -i "layer\|organize"
```

## Troubleshooting

### Snippets not loading

```bash
# Check permissions
ls -la /etc/wayexpand/snippets.d/

# Verify daemon sees them
systemctl --user status wayexpand-input-method.service
journalctl --user -u wayexpand-input-method -n 50
```

### Duplicate trigger errors

```bash
# Find conflicting triggers
grep -rn "^trigger = " /etc/wayexpand/snippets.d/ | sort | uniq -d

# Also check user layer for conflicts
grep -rn "^trigger = " ~/.config/wayexpand/snippets.d/ 2>/dev/null
```

### Ansible reload not working

```bash
# Manual reload for testing
systemctl --user restart wayexpand-input-method.service
wayexpand-daemon --source=input-method
```

## Security Notes

- Organization snippets are root-owned and world-readable (0644)
- Users cannot modify organization snippets (read-only)
- User personal snippets remain at ~/.config/wayexpand/snippets.d (user-owned)
- No credentials in snippet files; use secret providers (Vault, 1Password, etc.)
- Ansible playbook should be restricted to authorized deployment systems

## Advanced: Dynamic Snippet Generation

Generate snippets from external data:

```yaml
---
- hosts: sre-team
  tasks:
    - name: Generate snippets from CMDB
      template:
        src: cmdb-snippets.j2
        dest: /etc/wayexpand/snippets.d/99-cmdb.toml
        owner: root
        group: root
        mode: '0644'
      vars:
        services: "{{ hostvars['cmdb.internal'] | selectattr('type', 'eq', 'service') }}"

    - name: Reload daemon
      systemd:
        name: wayexpand-input-method.service
        state: restarted
        scope: user
      become: yes
      become_user: alice
```

## See Also

- [Fleet Configuration Guide](FLEET_CONFIG.md)
- [Puppet Integration](PUPPET_INTEGRATION.md)
- [Secret Management](SECRET_MANAGEMENT.md)
- [Organization Policy](ORGANIZATION_POLICY.md)
