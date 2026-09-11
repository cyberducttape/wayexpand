#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-doctor-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM

config_path="$test_root/expansions.toml"
printf '%s\n' \
    '[[expansion]]' \
    'trigger = ":doctor"' \
    'replacement = "ok"' >"$config_path"
chmod 0600 "$config_path"

WAYEXPAND_CONFIG="$config_path" \
    "$project_dir/target/debug/wayexpand" doctor >"$test_root/valid.out"
grep -F 'Config validation: OK' "$test_root/valid.out" >/dev/null

"$project_dir/target/debug/wayexpand" set-mode :doctor word-boundary "$config_path" \
    >"$test_root/set-mode.out"
grep -F 'word-boundary :doctor' "$test_root/set-mode.out" >/dev/null
grep -F 'match_mode = "word-boundary"' "$config_path" >/dev/null

"$project_dir/target/debug/wayexpand" doctor "$config_path" >"$test_root/valid-positional.out"
grep -F 'Config validation: OK' "$test_root/valid-positional.out" >/dev/null

symlink_path="$test_root/config-link.toml"
ln -s "$config_path" "$symlink_path"
"$project_dir/target/debug/wayexpand" doctor "$symlink_path" >"$test_root/valid-symlink.out"
grep -F 'Config validation: OK' "$test_root/valid-symlink.out" >/dev/null

control_parent="$test_root/control-parent"
mkdir "$control_parent"
chmod 0777 "$control_parent"
if WAYEXPAND_SOCKET="$control_parent/wayexpand.sock" \
    "$project_dir/target/debug/wayexpand" doctor "$config_path" \
    >"$test_root/insecure-control.out" 2>&1; then
    printf '%s\n' 'doctor accepted an insecure control-socket parent' >&2
    exit 1
fi
grep -F 'Control socket warning: parent is writable' "$test_root/insecure-control.out" >/dev/null

printf '%s\n' '[[expansion]]' 'trigger = ' >"$config_path"
if WAYEXPAND_CONFIG="$config_path" \
    "$project_dir/target/debug/wayexpand" doctor >"$test_root/invalid.out" 2>&1; then
    printf '%s\n' 'doctor accepted an invalid configuration' >&2
    exit 1
fi
grep -F 'Config validation: FAILED' "$test_root/invalid.out" >/dev/null

printf '%s\n' \
    '[[expansion]]' \
    'trigger = "sensitive-trigger"' \
    'replacement = "one"' \
    '[[expansion]]' \
    'trigger = "sensitive-trigger"' \
    'replacement = "two"' >"$config_path"
if WAYEXPAND_CONFIG="$config_path" \
    "$project_dir/target/debug/wayexpand" doctor >"$test_root/duplicate.out" 2>&1; then
    printf '%s\n' 'doctor accepted duplicate triggers' >&2
    exit 1
fi
grep -F 'Config validation: FAILED' "$test_root/duplicate.out" >/dev/null
if grep -F 'sensitive-trigger' "$test_root/duplicate.out" >/dev/null; then
    printf '%s\n' 'doctor leaked trigger contents' >&2
    exit 1
fi

if WAYEXPAND_CONFIG="$config_path" \
    "$project_dir/target/debug/wayexpand" test ':trigger' >"$test_root/test-invalid.out" 2>&1; then
    printf '%s\n' 'test command accepted an invalid configuration' >&2
    exit 1
fi
grep -F 'configuration invalid' "$test_root/test-invalid.out" >/dev/null
if grep -F 'sensitive-trigger' "$test_root/test-invalid.out" >/dev/null; then
    printf '%s\n' 'test command leaked trigger contents' >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ] && id nobody >/dev/null 2>&1; then
    untrusted_parent="$test_root/untrusted-parent"
    untrusted_config="$untrusted_parent/expansions.toml"
    mkdir "$untrusted_parent"
    cp "$config_path" "$untrusted_config"
    chmod 0755 "$untrusted_parent"
    chown "$(id -u nobody):$(id -g nobody)" "$untrusted_parent"
    if "$project_dir/target/debug/wayexpand" doctor "$untrusted_config" \
        >"$test_root/untrusted-parent.out" 2>&1; then
        printf '%s\n' 'doctor accepted an untrusted configuration parent' >&2
        exit 1
    fi
    grep -F 'parent directory owner is not trusted' "$test_root/untrusted-parent.out" >/dev/null

    chmod 1777 "$untrusted_parent"
    if "$project_dir/target/debug/wayexpand" doctor "$untrusted_config" \
        >"$test_root/untrusted-sticky-parent.out" 2>&1; then
        printf '%s\n' 'doctor accepted an untrusted sticky configuration parent' >&2
        exit 1
    fi
    grep -F 'parent directory owner is not trusted' "$test_root/untrusted-sticky-parent.out" >/dev/null
fi

printf '%s\n' 'doctor exit-status test passed'
