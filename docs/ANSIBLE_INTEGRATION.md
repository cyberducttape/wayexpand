# Ansible Integration for WayExpand Fleet Configuration

Deploy company-wide WayExpand snippets across your infrastructure using Ansible, without owning users' personal configurations.

## Architecture

WayExpand loads snippets from three layers in order:

1. **Organization layer:** `/etc/wayexpand/snippets.d/` (root-owned, managed by Ansible)
2. **User layer:** `~/.config/wayexpand/snippets.d/` (user personal, not touched by Ansible)
3. **Pack layer:** `~/.local/share/wayexpand/packs/` (optional curated packs)

Each layer is a directory of `.toml` files. Within a layer, files are loaded alphabetically. Duplicate triggers are rejected with source provenance.

How a running daemon picks up changes:

- **Snippet files** (`/etc/wayexpand/snippets.d/`): the shipped user services
  run the daemon in fleet mode, which watches these directories and reloads
  automatically. A reload that fails validation is refused and the last good
  configuration stays active. No restart is needed.
- **Organization policy** (`/etc/wayexpand/policy.toml`): read only when the
  daemon starts. A policy change needs a restart of running user services.

## Ansible Role Example

### Directory Structure

```
wayexpand-snippets/
├── tasks/
│   └── main.yml
├── handlers/
│   └── main.yml
├── files/
│   ├── 01-sre-core.toml
│   ├── 02-kubernetes.toml
│   ├── 03-incident-response.toml
│   └── policy.toml
├── templates/
│   └── organization.toml.j2
├── vars/
│   └── main.yml
└── meta/
    └── main.yml
```

### tasks/main.yml

Snippet tasks have no handler: running daemons reload them on their own. Only
a policy change restarts services, and only for desktop users whose user
manager is running.

```yaml
---
- name: Ensure the organization snippet directory exists
  ansible.builtin.file:
    path: /etc/wayexpand/snippets.d
    state: directory
    owner: root
    group: root
    mode: '0755'

- name: Deploy organization snippet packs
  ansible.builtin.copy:
    src: "{{ item }}"
    dest: /etc/wayexpand/snippets.d/
    owner: root
    group: root
    mode: '0644'
  loop:
    - 01-sre-core.toml
    - 02-kubernetes.toml
    - 03-incident-response.toml

- name: Deploy templated organization snippets
  ansible.builtin.template:
    src: organization.toml.j2
    dest: /etc/wayexpand/snippets.d/organization.toml
    owner: root
    group: root
    mode: '0644'

# User services read the policy as the desktop user, so it must be readable
# (0644). It must not be group- or world-writable. A policy that exists but
# cannot be read stops the daemon from starting.
- name: Deploy organization policy
  ansible.builtin.copy:
    src: policy.toml
    dest: /etc/wayexpand/policy.toml
    owner: root
    group: root
    mode: '0644'
  notify: Restart WayExpand for active desktop users

- name: Look up desktop user home directories
  ansible.builtin.getent:
    database: passwd

# The merged result depends on each user's personal snippets, so validate as
# each user. Users without a personal configuration are skipped.
- name: Validate the merged fleet configuration for each desktop user
  ansible.builtin.command:
    argv: [wayexpand, validate, --fleet, --json]
    removes: "{{ getent_passwd[item][4] }}/.config/wayexpand/expansions.toml"
  become: true
  become_user: "{{ item }}"
  loop: "{{ wayexpand_desktop_users }}"
  changed_when: false

# A user service can only be managed while that user's systemd manager and
# session bus are running. Query logind instead of assuming every account has one.
- name: Query logind for desktop user sessions
  ansible.builtin.command:
    argv: [loginctl, show-user, "{{ item }}", --property=State, --property=RuntimePath]
  loop: "{{ wayexpand_desktop_users }}"
  register: wayexpand_sessions
  changed_when: false
  failed_when: false

- name: Record desktop users with a running user manager
  ansible.builtin.set_fact:
    wayexpand_active_sessions: >-
      {{ (wayexpand_active_sessions | default([])) + [{
           'user': item.item,
           'runtime_dir': item.stdout | regex_search('(?m)^RuntimePath=(.+)$', '\\1') | first
         }] }}
  loop: "{{ wayexpand_sessions.results }}"
  loop_control:
    label: "{{ item.item }}"
  when:
    - item.rc == 0
    - item.stdout is search('(?m)^State=(active|online)$')
    - item.stdout is search('(?m)^RuntimePath=.+$')
```

### handlers/main.yml

Handlers run after all tasks, so services restart only after every user's
merged configuration has validated. `try-restart` restarts a service that is
already running and leaves stopped or never-enabled services alone.

```yaml
---
- name: Restart WayExpand for active desktop users
  ansible.builtin.command:
    argv:
      - systemctl
      - --user
      - try-restart
      - wayexpand-input-method.service
      - wayexpand-evdev.service
  become: true
  become_user: "{{ item.user }}"
  environment:
    XDG_RUNTIME_DIR: "{{ item.runtime_dir }}"
    DBUS_SESSION_BUS_ADDRESS: "unix:path={{ item.runtime_dir }}/bus"
  loop: "{{ wayexpand_active_sessions | default([]) }}"
  loop_control:
    label: "{{ item.user }}"
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

List the desktop accounts that run WayExpand explicitly. Do not derive them
from the passwd database: system and service accounts have no desktop session.

```yaml
---
company_name: "ACME Inc"
organization_id: "acme-prod-2026"
oncall_contact: "oncall@acme.example.com"
wayexpand_desktop_users:
  - alice
  - bob
```

### files/policy.toml

```toml
[organization]
safe_mode = true
disable_hotkeys = false
disable_title_matching = false
max_replacement_size = 65536
allowed_backends = ["input-method-v2", "libei"]
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

The role deploys `files/policy.toml`, shown above, with `safe_mode = true`.
See [Organization Policy](ORGANIZATION_POLICY.md) for every field. Keep the
file root-owned and `0644`: a `0600` policy cannot be read by the user
services and stops them from starting.

## Example Snippet Files

### 01-sre-core.toml

```toml
# Core SRE/DevOps snippets

[[expansion]]
trigger = ":logme"
replacement = "journalctl --user -u wayexpand-input-method.service -n 50 -f"
description = "Tail recent WayExpand logs"
category = "logging"

[[expansion]]
trigger = ":wxstatus"
replacement = "wayexpand status --json"
description = "Insert the WayExpand status command"
category = "operations"

[[expansion]]
trigger = ":test-snippet"
replacement = "wayexpand validate --fleet --json"
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
wayexpand fleet status --json | jq '{files_loaded, expansion_count, hotkey_count, layers}'

# Show provenance
journalctl --user -u wayexpand-input-method.service | grep -i "layer\|organize"
```

## Troubleshooting

### Snippets not loading

```bash
# Check permissions
ls -la /etc/wayexpand/snippets.d/

# Verify daemon sees them
systemctl --user status wayexpand-input-method.service
journalctl --user -u wayexpand-input-method.service -n 50
```

### Duplicate trigger errors

```bash
# Find conflicting triggers
grep -rn "^trigger = " /etc/wayexpand/snippets.d/ | sort | uniq -d

# Also check user layer for conflicts
grep -rn "^trigger = " ~/.config/wayexpand/snippets.d/ 2>/dev/null
```

### Changes not picked up

```bash
# Ask the running daemon to reload now (it normally does so on its own)
wayexpand reload

# A refused reload names the problem; the last good configuration stays active
journalctl --user -u wayexpand-input-method.service -n 50 | grep -i reload

# Policy changes need a restart of the running service
systemctl --user try-restart wayexpand-input-method.service
```

Do not start `wayexpand-daemon` by hand while the user service is running: two
daemons would compete for the same input and control socket.

## Security Notes

- Organization snippets and `policy.toml` are root-owned and world-readable (0644); they must never be group- or world-writable
- Users cannot modify organization snippets (read-only)
- User personal snippets remain at ~/.config/wayexpand/snippets.d (user-owned)
- No credentials in snippet files; use secret providers (Vault, 1Password, etc.)
- Avoid command snippets that control WayExpand itself (for example restarting its service): the command runs inside the expansion it would interrupt
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
```

Running daemons reload the generated file automatically. Run the role's
validation tasks afterwards so a duplicate trigger is reported by the play
rather than only in each user's journal.

## See Also

- [Fleet Configuration Guide](FLEET_CONFIG.md)
- [Puppet Integration](PUPPET_INTEGRATION.md)
- [Secret Management](SECRET_MANAGEMENT.md)
- [Organization Policy](ORGANIZATION_POLICY.md)
