# Puppet Integration for WayExpand Fleet Configuration

Deploy and manage WayExpand organization snippets using Puppet, maintaining strict separation between managed organization snippets and user personal configurations.

## Architecture

WayExpand fleet configuration uses three independent layers:

```
/etc/wayexpand/snippets.d/          ← Puppet-managed (organization snippets)
  ├── 01-sre-core.toml
  ├── 02-kubernetes.toml
  └── 03-incident-response.toml

~/.config/wayexpand/snippets.d/      ← User-managed (personal snippets)
  ├── my-snippets.toml
  └── work-specific.toml

~/.local/share/wayexpand/packs/      ← Optional curated packs
```

Each layer is loaded independently. Files within a layer are merged alphabetically. Duplicate triggers between layers are rejected with clear error messages showing source provenance.

How a running daemon picks up changes:

- **Snippet files** (`/etc/wayexpand/snippets.d/`): the shipped user services
  run the daemon in fleet mode, which watches these directories and reloads
  automatically. A reload that fails validation is refused and the last good
  configuration stays active. No restart is needed.
- **Organization policy** (`/etc/wayexpand/policy.toml`): read only when the
  daemon starts. A policy change needs a restart of running user services.

## Puppet Module Example

### Directory Structure

```
wayexpand/
├── files/
│   ├── 01-sre-core.toml
│   ├── 02-kubernetes.toml
│   ├── 03-incident-response.toml
│   ├── policy.toml
│   └── wayexpand-restart-active.sh
├── templates/
│   └── organization.toml.epp
├── manifests/
│   ├── init.pp
│   ├── config.pp
│   └── params.pp
└── README.md
```

### manifests/params.pp

```puppet
# wayexpand/manifests/params.pp

class wayexpand::params {
  $config_dir = '/etc/wayexpand/snippets.d'
  $config_owner = 'root'
  $config_group = 'root'
  $config_mode = '0755'
  
  $snippet_owner = 'root'
  $snippet_group = 'root'
  $snippet_mode = '0644'
  
  # Desktop accounts that run WayExpand. List them explicitly: system and
  # service accounts have no desktop session or user manager.
  $desktop_users = []

  # Organization metadata for templated snippets
  $organization_name = 'ACME Inc'
  $organization_id = 'acme-prod-2026'
  $oncall_contact = 'oncall@acme.example.com'
}
```

### manifests/config.pp

```puppet
# wayexpand/manifests/config.pp

class wayexpand::config (
  String $config_dir = $wayexpand::params::config_dir,
  String $config_owner = $wayexpand::params::config_owner,
  String $config_group = $wayexpand::params::config_group,
  String $config_mode = $wayexpand::params::config_mode,
  String $snippet_owner = $wayexpand::params::snippet_owner,
  String $snippet_group = $wayexpand::params::snippet_group,
  String $snippet_mode = $wayexpand::params::snippet_mode,
) inherits wayexpand::params {

  # Ensure organization snippet directory exists
  file { $config_dir:
    ensure => directory,
    owner  => $config_owner,
    group  => $config_group,
    mode   => $config_mode,
  }

  # Deploy core SRE snippets
  file { "${config_dir}/01-sre-core.toml":
    ensure  => file,
    owner   => $snippet_owner,
    group   => $snippet_group,
    mode    => $snippet_mode,
    source  => 'puppet:///modules/wayexpand/01-sre-core.toml',
    require => File[$config_dir],
  }

  # Deploy Kubernetes snippets
  file { "${config_dir}/02-kubernetes.toml":
    ensure  => file,
    owner   => $snippet_owner,
    group   => $snippet_group,
    mode    => $snippet_mode,
    source  => 'puppet:///modules/wayexpand/02-kubernetes.toml',
    require => File[$config_dir],
  }

  # Deploy incident response snippets
  file { "${config_dir}/03-incident-response.toml":
    ensure  => file,
    owner   => $snippet_owner,
    group   => $snippet_group,
    mode    => $snippet_mode,
    source  => 'puppet:///modules/wayexpand/03-incident-response.toml',
    require => File[$config_dir],
  }

  # Deploy templated organization config
  file { "${config_dir}/organization.toml":
    ensure  => file,
    owner   => $snippet_owner,
    group   => $snippet_group,
    mode    => $snippet_mode,
    content => epp('wayexpand/organization.toml.epp', {
      organization_name => $wayexpand::params::organization_name,
      organization_id   => $wayexpand::params::organization_id,
      oncall_contact    => $wayexpand::params::oncall_contact,
    }),
    require => File[$config_dir],
  }

  # User services read the policy as the desktop user, so it must be readable
  # (0644). It must not be group- or world-writable. A policy that exists but
  # cannot be read stops the daemon from starting.
  file { '/etc/wayexpand/policy.toml':
    ensure => file,
    owner  => 'root',
    group  => 'root',
    mode   => '0644',
    source => 'puppet:///modules/wayexpand/policy.toml',
  }
}
```

### manifests/init.pp

```puppet
# wayexpand/manifests/init.pp

class wayexpand (
  Boolean $manage_config = true,
  Boolean $validate_config = true,
  Boolean $restart_on_policy_change = true,
  Array[String] $desktop_users = $wayexpand::params::desktop_users,
) inherits wayexpand::params {

  # Include configuration management
  if $manage_config {
    include wayexpand::config
  }

  # The merged result depends on each user's personal snippets, so validate
  # as each desktop user. Users without a personal configuration are skipped.
  # Snippet changes need no restart: running daemons reload them on their own.
  if $validate_config {
    $desktop_users.each |String $user| {
      exec { "wayexpand-validate-fleet-${user}":
        provider    => shell,
        user        => $user,
        command     => "HOME=\"\$(getent passwd ${user} | cut -d: -f6)\"; export HOME; test ! -f \"\$HOME/.config/wayexpand/expansions.toml\" || /usr/bin/wayexpand validate --fleet --json",
        refreshonly => true,
        subscribe   => Class['wayexpand::config'],
        require     => Package['wayexpand'],
        before      => Exec['wayexpand-restart-active-users'],
      }
    }
  }

  file { '/usr/local/sbin/wayexpand-restart-active.sh':
    ensure => file,
    owner  => 'root',
    group  => 'root',
    mode   => '0755',
    source => 'puppet:///modules/wayexpand/wayexpand-restart-active.sh',
  }

  # The policy is read only at daemon startup. Restart services that are
  # already running, and only for users whose user manager is running.
  exec { 'wayexpand-restart-active-users':
    command     => "/usr/local/sbin/wayexpand-restart-active.sh ${desktop_users.join(' ')}",
    refreshonly => true,
    noop        => !$restart_on_policy_change,
    subscribe   => File['/etc/wayexpand/policy.toml'],
    require     => File['/usr/local/sbin/wayexpand-restart-active.sh'],
  }
}
```

### templates/organization.toml.epp

```epp
# Organization-wide WayExpand snippets
# Managed by Puppet - do not edit manually
# Generated for: <%= $facts['hostname'] %>
# Timestamp: <%= $facts['system_uptime']['seconds'] %>

[[expansion]]
trigger = ":companyname"
replacement = "<%= $organization_name %>"
description = "Insert organization name"
category = "organization"

[[expansion]]
trigger = ":orgid"
replacement = "<%= $organization_id %>"
description = "Insert organization ID"
category = "organization"

[[expansion]]
trigger = ":oncall"
replacement = "<%= $oncall_contact %>"
description = "Current on-call contact"
category = "operations"

[[expansion]]
trigger = ":hostname"
replacement = "<%= $facts['hostname'] %>"
description = "This machine's hostname"
category = "system"

[[expansion]]
trigger = ":env"
replacement = "prod"
description = "Environment (prod/staging/dev)"
category = "environment"
app_filter = ["app_id_glob:*gitlab*", "app_id_glob:*jenkins*", "app_id_glob:*terraform*"]
```

### files/policy.toml

```toml
[organization]
safe_mode = true
max_replacement_size = 65536
allowed_backends = ["input-method-v2", "libei"]
```

See [Organization Policy](ORGANIZATION_POLICY.md) for every field.

### files/wayexpand-restart-active.sh

Installed as `/usr/local/sbin/wayexpand-restart-active.sh` and run only after a
policy change. A user service can only be managed while that user's systemd
manager and session bus are running, so each user is checked with logind
first. `try-restart` leaves stopped or never-enabled services alone.

```sh
#!/bin/sh
# Restart running WayExpand services for the listed desktop users.
set -eu

status=0
for user in "$@"; do
    state=$(loginctl show-user "$user" --property=State --value 2>/dev/null) || continue
    case "$state" in
        active|online) ;;
        *) continue ;;
    esac
    runtime_dir=$(loginctl show-user "$user" --property=RuntimePath --value)
    [ -S "$runtime_dir/bus" ] || continue
    echo "Restarting running WayExpand services for $user"
    runuser -u "$user" -- env \
        XDG_RUNTIME_DIR="$runtime_dir" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime_dir/bus" \
        systemctl --user try-restart \
        wayexpand-input-method.service wayexpand-evdev.service || status=1
done
exit "$status"
```

## Hiera Configuration

### common.yaml

```yaml
---
wayexpand::manage_config: true
wayexpand::validate_config: true
wayexpand::restart_on_policy_change: true
wayexpand::desktop_users:
  - alice
  - bob
wayexpand::organization_name: "ACME Inc"
wayexpand::organization_id: "acme-prod-2026"
wayexpand::oncall_contact: "oncall@acme.example.com"
```

### node/prod-sre-team.yaml

```yaml
---
wayexpand::manage_config: true
wayexpand::validate_config: true
wayexpand::restart_on_policy_change: true
wayexpand::organization_name: "ACME Production"
wayexpand::organization_id: "acme-prod-2026"
wayexpand::oncall_contact: "sre-oncall@acme.example.com"
```

## Usage

### Apply to all nodes

```bash
puppet apply --hiera_config=/etc/puppet/hiera.yaml --modulepath=/etc/puppet/modules
```

### Apply specific profile

```bash
puppet agent -t --tags wayexpand
```

### Test changes (no-op)

```bash
puppet agent -t --noop --tags wayexpand
```

## Verification

### Check applied configuration

```bash
# View organization snippets
cat /etc/wayexpand/snippets.d/*.toml

# Check file permissions (should be 0644 owned by root)
ls -la /etc/wayexpand/snippets.d/

# Verify Puppet resource state
puppet resource file /etc/wayexpand/snippets.d/01-sre-core.toml
```

### Test fleet merge

```bash
# Run locally on managed node
wayexpand fleet status --json | jq '{files_loaded, expansion_count, hotkey_count, layers}'

# Should show something like:
# {
#   "total_files_loaded": 4,
#   "total_expansions": 23,
#   "total_hotkeys": 0,
#   "layers_applied": [
#     "organization (/etc/wayexpand/snippets.d)",
#     "user (/home/alice/.config/wayexpand/snippets.d)"
#   ]
# }
```

### Check Puppet reports

```bash
# On Puppet master
puppet report list
puppet report show --from-file=/var/lib/puppet/reports/node.example.com/latest.yaml

# Check for wayexpand resources
puppet resource changes | grep wayexpand
```

## Troubleshooting

### Validation fails

```bash
# Check for duplicate triggers
grep -rn "^trigger = " /etc/wayexpand/snippets.d/ | cut -d: -f3 | sort | uniq -d

# View validation output (run as the affected desktop user)
wayexpand validate --fleet --json 2>&1
```

### Changes not picked up

```bash
# As the desktop user: ask the running daemon to reload now
wayexpand reload

# A refused reload names the problem; the last good configuration stays active
journalctl --user -u wayexpand-input-method.service -n 20 | grep -i reload

# As root, after a policy change
/usr/local/sbin/wayexpand-restart-active.sh alice
```

### Puppet runs fail

```bash
# Validate manifest syntax
puppet parser validate /etc/puppet/modules/wayexpand/manifests/init.pp

# Check Puppet logs
tail -f /var/log/puppet/puppet.log

# Run in debug mode
puppet agent -t --debug --tags wayexpand
```

## Advanced: Dynamic Snippets from External Data

### Fetch snippets from Git

```puppet
class wayexpand::git_snippets (
  String $repo_url = 'https://git.internal/wayexpand-snippets.git',
  String $branch = 'main',
) {
  vcsrepo { '/opt/wayexpand-snippets':
    ensure   => present,
    provider => git,
    source   => $repo_url,
    revision => $branch,
    require  => Package['git'],
    notify   => Exec['deploy-git-snippets'],
  }

  exec { 'deploy-git-snippets':
    command     => 'cp /opt/wayexpand-snippets/*.toml /etc/wayexpand/snippets.d/',
    refreshonly => true,
    require     => Vcsrepo['/opt/wayexpand-snippets'],
  }
}
```

### Generate from CMDB

```puppet
define wayexpand::snippet_from_cmdb (
  String $trigger,
  String $replacement,
  String $category = 'generated',
) {
  file { "/etc/wayexpand/snippets.d/cmdb-${name}.toml":
    ensure  => file,
    content => @("EOT"/L),
      # Auto-generated from CMDB: ${name}
      [[expansion]]
      trigger = "${trigger}"
      replacement = "${replacement}"
      category = "${category}"
      | EOT
    owner   => root,
    group   => root,
    mode    => '0644',
  }
}
```

Running daemons reload generated files automatically. Declare these resources
inside `wayexpand::config`, or have them notify the per-user
`wayexpand-validate-fleet-*` execs, so a duplicate trigger is reported by the
Puppet run rather than only in each user's journal.

## See Also

- [Fleet Configuration Guide](FLEET_CONFIG.md)
- [Ansible Integration](ANSIBLE_INTEGRATION.md)
- [Organization Policy](ORGANIZATION_POLICY.md)
- [Secret Management](SECRET_MANAGEMENT.md)
