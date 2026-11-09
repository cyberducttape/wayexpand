#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-apt-sources.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM
mkdir -p "$temporary_root/sources.list.d"
cat > "$temporary_root/sources.list" <<'EOF'
deb http://archive.ubuntu.com/ubuntu noble main
deb http://packages.example.net/ubuntu stable main
EOF
cat > "$temporary_root/sources.list.d/ubuntu.sources" <<'EOF'
Types: deb
URIs: http://security.ubuntu.com/ubuntu
Suites: noble-security
Components: main
EOF

sh "$script_dir/use-https-ubuntu-mirrors.sh" "$temporary_root"
grep -Fqx 'deb https://archive.ubuntu.com/ubuntu noble main' "$temporary_root/sources.list"
grep -Fqx 'deb http://packages.example.net/ubuntu stable main' "$temporary_root/sources.list"
grep -Fqx 'URIs: https://security.ubuntu.com/ubuntu' \
    "$temporary_root/sources.list.d/ubuntu.sources"
grep -Fqx 'Acquire::Retries "3";' \
    "$temporary_root/apt.conf.d/99wayexpand-ci-network"
grep -Fqx 'Acquire::https::Timeout "20";' \
    "$temporary_root/apt.conf.d/99wayexpand-ci-network"

printf '%s\n' 'Ubuntu APT mirror rewrite test passed'
