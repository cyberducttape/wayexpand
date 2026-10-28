#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-build-identity.XXXXXXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
mkdir -p "$test_root/debian" "$test_root/scripts"
cp "$project_dir/scripts/load-build-identity.sh" "$test_root/scripts/"
cat >"$test_root/Cargo.toml" <<'EOF'
version = "1.3.3"
EOF

run_helper() {
    local inherited_sha=${1:-}
    local inherited_version=${2:-}
    (
        cd "$test_root"
        # shellcheck disable=SC2016 # The code must expand inside the child shell.
        env WAYEXPAND_BUILD_SHA="$inherited_sha" \
            WAYEXPAND_BUILD_RELEASE_VERSION="$inherited_version" \
            bash -c '. scripts/load-build-identity.sh; printf "%s\n%s\n" "$WAYEXPAND_BUILD_SHA" "${WAYEXPAND_BUILD_RELEASE_VERSION:-}"'
    )
}

expect_success() {
    local label=$1 expected_sha=$2 expected_version=$3
    shift 3
    local output
    output=$(run_helper "$@") || {
        printf 'build identity rejected %s\n' "$label" >&2
        exit 1
    }
    if [ "$output" != "$(printf '%s\n%s' "$expected_sha" "$expected_version")" ]; then
        printf 'unexpected build identity for %s: %s\n' "$label" "$output" >&2
        exit 1
    fi
}

expect_failure() {
    local label=$1
    shift
    if run_helper "$@" >/dev/null 2>&1; then
        printf 'build identity unexpectedly accepted %s\n' "$label" >&2
        exit 1
    fi
}

printf '%s\n' 'abcdef123456' >"$test_root/debian/wayexpand-build-sha"
printf '%s\n' '1.3.3' >"$test_root/debian/wayexpand-build-version"
expect_success 'exact release stamp' abcdef123456 1.3.3 '' ''

rm "$test_root/debian/wayexpand-build-version"
expect_success 'development stamp' abcdef123456 '' '' ''

printf '%s\n' 'bad' >"$test_root/debian/wayexpand-build-sha"
expect_failure 'short commit stamp' '' ''
printf '%s\n' 'abcdef123456' >"$test_root/debian/wayexpand-build-sha"
printf '%s\n' '1.3.4' >"$test_root/debian/wayexpand-build-version"
expect_failure 'release version mismatch' '' ''
printf '%s\n' '1.3.3' >"$test_root/debian/wayexpand-build-version"
expect_failure 'inherited commit mismatch' 123456abcdef 1.3.3

printf '%s\n' 'RPM build identity validates release, development, and mismatch cases'
