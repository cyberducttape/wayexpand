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

## Puppet Module Example

### Directory Structure

```
wayexpand/
├── files/
│   ├── 01-sre-core.toml
│   ├── 02-kubernetes.toml
│   └── 03-incident-response.toml
├── templates/
│   └── organization.toml.epp
├── manifests/
│   ├── init.pp
│   ├── config.pp
│   ├── service.pp
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
  
  $service_names = [
    'wayexpand-input-method.service',
    'wayexpand-evdev.service',
  ]
  
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
}
```

### manifests/init.pp

```puppet
# wayexpand/manifests/init.pp

class wayexpand (
  Boolean $manage_config = true,
  Boolean $validate_config = true,
  Boolean $reload_service = true,
) inherits wayexpand::params {

  # Include configuration management
  if $manage_config {
    include wayexpand::config
  }

  # Validate merged fleet configuration after deployment
  if $validate_config {
    exec { 'wayexpand-validate-fleet':
      command     => '/usr/bin/wayexpand validate --fleet --json',
      refreshonly => true,
      onlyif      => '/usr/bin/test -d /etc/wayexpand/snippets.d',
      subscribe   => [
        File["${wayexpand::params::config_dir}/01-sre-core.toml"],
        File["${wayexpand::params::config_dir}/02-kubernetes.toml"],
        File["${wayexpand::params::config_dir}/03-incident-response.toml"],
        File["${wayexpand::params::config_dir}/organization.toml"],
      ],
      require     => Package['wayexpand'],
      notify      => Exec['wayexpand-reload-all-users'],
    }
  }

  # Reload daemon for all active users
  if $reload_service {
    exec { 'wayexpand-reload-all-users':
      command     => '/usr/local/bin/wayexpand-reload-all-users.sh',
      refreshonly => true,
      onlyif      => '/usr/bin/test -f /usr/local/bin/wayexpand-reload-all-users.sh',
    }
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
app_filter = ["gitlab", "jenkins", "terraform"]
```

### files/reload-wrapper.sh

Place this at `/usr/local/bin/wayexpand-reload-all-users.sh`:

```bash
#!/bin/bash
# Reload WayExpand daemon for all active users
# Called by Puppet after organization snippet updates

set -e

ACTIVE_USERS=$(who | awk '{print $1}' | sort | uniq)

for user in $ACTIVE_USERS; do
  if [[ "$user" != "root" && ! "$user" =~ ^(sync|shutdown|halt|nobody)$ ]]; then
    echo "Reloading WayExpand for user: $user"
    su - "$user" -c "systemctl --user restart wayexpand-input-method.service" || true
  fi
done

echo "WayExpand fleet reload complete"
```

## Hiera Configuration

### common.yaml

```yaml
---
wayexpand::manage_config: true
wayexpand::validate_config: true
wayexpand::reload_service: true
wayexpand::organization_name: "ACME Inc"
wayexpand::organization_id: "acme-prod-2026"
wayexpand::oncall_contact: "oncall@acme.example.com"
```

### node/prod-sre-team.yaml

```yaml
---
wayexpand::manage_config: true
wayexpand::validate_config: true
wayexpand::reload_service: true
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

# View validation output
wayexpand validate --fleet --json 2>&1
```

### Daemon doesn't reload

```bash
# Check if reload script exists and is executable
ls -la /usr/local/bin/wayexpand-reload-all-users.sh

# Test manually
/usr/local/bin/wayexpand-reload-all-users.sh

# Check journalctl
journalctl -u wayexpand-input-method.service -n 20
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
    notify      => Exec['wayexpand-reload-all-users'],
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
    notify  => Exec['wayexpand-reload-all-users'],
  }
}
```

## See Also

- [Fleet Configuration Guide](FLEET_CONFIG.md)
- [Ansible Integration](ANSIBLE_INTEGRATION.md)
- [Organization Policy](ORGANIZATION_POLICY.md)
- [Secret Management](SECRET_MANAGEMENT.md)
