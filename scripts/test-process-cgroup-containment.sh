#!/bin/sh
# Exercise the setsid escape regression inside the same delegated cgroup
# topology used by the installed WayExpand systemd services.
set -eu

if ! command -v systemd-run >/dev/null 2>&1; then
    echo "systemd-run is required for cgroup containment integration testing" >&2
    exit 1
fi

unit="wayexpand-cgroup-test-$$"
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
systemd-run --user --wait --pipe --collect --unit="$unit" \
    -p Delegate=yes \
    -p ProtectControlGroups=no \
    -p ProtectKernelTunables=yes \
    -p ReadWritePaths=/sys/fs/cgroup \
    -E WAYEXPAND_PROCESS_CGROUP=required \
    -E WAYEXPAND_REQUIRE_CGROUP_TEST=1 \
    sh -c 'cd "$1" && exec cargo test --locked -p wayexpand-process-supervisor tests::delegated_cgroup_cleanup_terminates_new_sessions -- --exact --nocapture' \
        wayexpand-cgroup-test "$repo_dir"
