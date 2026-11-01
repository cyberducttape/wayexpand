#!/bin/sh
# Exercise command cgroup containment inside a delegated cgroup topology
# matching the installed WayExpand systemd services:
#
# 1. a descendant that calls setsid(), and even one that migrates itself back
#    to the service cgroup, is terminated when its command is cleaned up;
# 2. ordinary commands and broker actions run under the units' sandbox
#    properties.
#
# Each test binary runs as the unit's only process, as the daemon does, so the
# service cgroup is dedicated and escapee sweeps are enabled.
set -eu

if ! command -v systemd-run >/dev/null 2>&1; then
    echo "systemd-run is required for cgroup containment integration testing" >&2
    exit 1
fi

repo_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

# Print the test executable that cargo builds for a package's library.
test_binary() {
    cargo test --locked --manifest-path "$repo_dir/Cargo.toml" -p "$1" --lib --no-run \
        --message-format=json 2>/dev/null |
        sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -n 1
}

supervisor=$(test_binary wayexpand-process-supervisor)
core=$(test_binary wayexpand-core)
broker=$(test_binary action-broker)
for binary in "$supervisor" "$core" "$broker"; do
    [ -x "$binary" ] || {
        echo "could not build test binary: $binary" >&2
        exit 1
    }
done

run_contained() {
    systemd-run --user --wait --pipe --collect --unit="wayexpand-cgroup-test-$$-$1" \
        -p Delegate=yes \
        -p ProtectControlGroups=no \
        -p ProtectKernelTunables=yes \
        -p ProtectSystem=strict \
        -p ProtectHome=read-only \
        -p PrivateTmp=yes \
        -p NoNewPrivileges=yes \
        -p ReadWritePaths=/sys/fs/cgroup \
        -E WAYEXPAND_PROCESS_CGROUP=required \
        -E WAYEXPAND_REQUIRE_CGROUP_TEST=1 \
        "$2" "$3" --exact --nocapture --test-threads=1
}

run_contained escape "$supervisor" \
    tests::delegated_cgroup_terminates_new_session_parent_migration
run_contained command "$core" engine::tests::commands_execute_when_not_disabled
run_contained broker "$broker" executor::tests::executor_timeout_kills_process
