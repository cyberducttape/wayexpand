#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
source "$project_dir/scripts/private-home-cleanup.sh"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-private-home.XXXXXX")
private_home="$test_root/private-home"
unrelated_home="$test_root/unrelated-home"
mkdir -m 700 "$private_home" "$unrelated_home"
private_pid=
unrelated_pid=
cleanup() {
    [ -z "$private_pid" ] || kill "$private_pid" 2>/dev/null || true
    [ -z "$unrelated_pid" ] || kill "$unrelated_pid" 2>/dev/null || true
    [ -z "$private_pid" ] || wait "$private_pid" 2>/dev/null || true
    [ -z "$unrelated_pid" ] || wait "$unrelated_pid" 2>/dev/null || true
    rm -rf "$test_root"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

env HOME="$private_home" sleep 60 &
private_pid=$!
env HOME="$unrelated_home" sleep 60 &
unrelated_pid=$!
sleep 0.1

reap_private_home_processes "$private_home"
set +e
wait "$private_pid"
private_status=$?
set -e
if [ "$private_status" -ne 143 ]; then
    printf 'private test process was not terminated by cleanup (status %s)\n' \
        "$private_status" >&2
    exit 1
fi
private_pid=
if ! kill -0 "$unrelated_pid" 2>/dev/null; then
    printf '%s\n' 'cleanup terminated a process outside the private HOME' >&2
    exit 1
fi

printf '%s\n' 'private HOME process cleanup passed'
