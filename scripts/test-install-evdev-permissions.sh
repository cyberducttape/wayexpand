#!/bin/sh
# CI-safe checks for install-evdev-permissions.sh: everything that does not
# require root or touch real system state (installing the rule for real,
# and modifying group membership, both need root and are not exercised
# here -- a human running this by hand with sudo is the real test for
# those).
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
script="$project_dir/scripts/install-evdev-permissions.sh"

"$script" --help >/dev/null

out=$("$script" --dry-run)
printf '%s' "$out" | grep -F 'This will:' >/dev/null
printf '%s' "$out" | grep -F '(dry run; no changes made)' >/dev/null
printf '%s' "$out" | grep -F 'SECURITY.md' >/dev/null
seat_out=$($script --access=active-seat --dry-run)
printf '%s' "$seat_out" | grep -F 'active-seat' >/dev/null
printf '%s' "$seat_out" | grep -F 'do not change input-group membership' >/dev/null

if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' "skipping non-root-rejection checks: already running as root" >&2
else
    if "$script" >/dev/null 2>&1; then
        printf '%s\n' "install-evdev-permissions.sh accepted a non-root apply" >&2
        exit 1
    fi
    if "$script" --uninstall >/dev/null 2>&1; then
        printf '%s\n' "install-evdev-permissions.sh accepted a non-root --uninstall" >&2
        exit 1
    fi
fi

if "$script" --bogus-flag >/dev/null 2>&1; then
    printf '%s\n' "install-evdev-permissions.sh accepted an unknown flag" >&2
    exit 1
fi

# Confirm it fails clearly when not run from the project tree / a release
# layout (no udev/71-wayexpand-evdev.rules next to it), instead of a bare
# "No such file" further down.
empty_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-evdev-perm-test.XXXXXX")
trap 'rm -rf "$empty_dir"' EXIT INT TERM
mkdir -p "$empty_dir/scripts"
cp "$script" "$empty_dir/scripts/"
if "$empty_dir/scripts/install-evdev-permissions.sh" --dry-run >"$empty_dir/out" 2>&1; then
    printf '%s\n' "install-evdev-permissions.sh ran without its udev rule file present" >&2
    exit 1
fi
grep -F 'not found' "$empty_dir/out" >/dev/null

printf '%s\n' "install-evdev-permissions.sh test passed"
