#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT

# Simulate root so the test reaches the container guard without requiring
# privilege. The package smoke script must reject a host even when its
# dangerous-operation opt-in variable is set.
mkdir "$temporary_dir/bin"
cat >"$temporary_dir/bin/id" <<'EOF'
#!/bin/sh
if [ "${1:-}" = -u ]; then
    printf '%s\n' 0
else
    exec /usr/bin/id "$@"
fi
EOF
chmod 0755 "$temporary_dir/bin/id"

if [ -e /.dockerenv ] || [ -e /run/.containerenv ]; then
    printf '%s\n' 'skipping host-only guard assertion inside a detected container'
    exit 0
fi

if PATH="$temporary_dir/bin:$PATH" \
    WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE=1 \
    sh "$project_dir/scripts/test-package-smoke.sh" "$temporary_dir/fake.deb" \
    >"$temporary_dir/stdout" 2>"$temporary_dir/stderr"; then
    printf '%s\n' 'package smoke test accepted the opt-in variable outside a container' >&2
    exit 1
fi
grep -Fx 'error: package smoke test requires a detected Docker or Podman container' \
    "$temporary_dir/stderr" >/dev/null
printf '%s\n' 'package smoke test refuses host execution even with explicit opt-in'
