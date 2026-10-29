#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-cert-processes.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
. "$project_dir/scripts/certification-processes.sh"

proc_root="$test_root/proc"
private_root="$test_root/private-run"
uid=$(id -u)
mkdir -p "$proc_root/101" "$proc_root/102" "$proc_root/103"

# Normal private-session children are identified by their isolated HOME.
printf 'HOME=%s/layout/home\000' "$private_root" >"$proc_root/101/environ"
printf '/usr/bin/xdg-desktop-portal\000' >"$proc_root/101/cmdline"

# KDE may delegate an editor to the host user manager, which supplies the
# ordinary HOME. Its unique private-document argument must still identify it.
printf 'HOME=/home/tester\000XDG_RUNTIME_DIR=/run/user/1000\000' \
    >"$proc_root/102/environ"
printf '/usr/bin/kate\000--new\000%s/layout/document.txt\000' \
    "$private_root" >"$proc_root/102/cmdline"

# A normal desktop editor is outside this test and must be left alone.
printf 'HOME=/home/tester\000' >"$proc_root/103/environ"
printf '/usr/bin/kate\000--new\000/home/tester/notes.txt\000' \
    >"$proc_root/103/cmdline"

found=$(certification_private_pids "$uid" "$private_root" "$proc_root")
printf '%s\n' "$found" | grep -Fx 101 >/dev/null
printf '%s\n' "$found" | grep -Fx 102 >/dev/null
if printf '%s\n' "$found" | grep -Fx 103 >/dev/null; then
    printf '%s\n' 'process cleanup matched an unrelated user editor' >&2
    exit 1
fi

printf '%s\n' 'certification process cleanup contract passed'
