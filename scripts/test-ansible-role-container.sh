#!/usr/bin/env bash
set -euo pipefail

cli=${1:-target/debug/wayexpand}
if ! command -v docker >/dev/null 2>&1; then
    printf '%s\n' 'error: docker is required for the disposable Ansible role test' >&2
    exit 1
fi

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-ansible-role.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT
project="$temporary_root/project"
role_directory="$project/roles/wayexpand-container-test"
script_directory=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$project/roles"

"$cli" new ansible-role \
    --set role=wayexpand-container-test \
    --output "$role_directory"
cat > "$project/site.yml" <<'EOF'
---
- name: Exercise the generated role in a disposable container
  hosts: localhost
  connection: local
  become: false
  roles:
    - wayexpand-container-test
EOF

# The image has no init system, so keep the role's restart handler from
# attempting to control a host service. Package installation and template
# application still run against the disposable Ubuntu target.
cat > "$role_directory/handlers/main.yml" <<'EOF'
---
- name: Restart nginx
  ansible.builtin.debug:
    msg: service restart requested
EOF

docker run --rm \
    --volume "$project:/project" \
    --volume "$script_directory/use-https-ubuntu-mirrors.sh:/tmp/use-https-ubuntu-mirrors.sh:ro" \
    --volume /etc/ssl/certs/ca-certificates.crt:/tmp/wayexpand-runner-ca.pem:ro \
    --env WAYEXPAND_APT_CA_FILE=/tmp/wayexpand-runner-ca.pem \
    --workdir /project \
    ubuntu:24.04 \
    bash -euc '
        export DEBIAN_FRONTEND=noninteractive
        sh /tmp/use-https-ubuntu-mirrors.sh
        apt-get update
        apt-get install -y --no-install-recommends ansible-core
        ansible-playbook --syntax-check -i localhost, site.yml
        first=$(ansible-playbook -i localhost, site.yml)
        printf "%s\n" "$first" | tail -n 5
        printf "%s\n" "$first" | grep -Eq "changed=[1-9][0-9]*"
        second=$(ansible-playbook -i localhost, site.yml)
        printf "%s\n" "$second" | tail -n 5
        printf "%s\n" "$second" | grep -q "changed=0"
    '
