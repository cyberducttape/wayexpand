#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak-restarts.XXXXXX")
runtime_tmp="$test_root/runtime"
report_dir="$test_root/report"
mkdir -m 0700 "$runtime_tmp"
trap 'rm -rf "$test_root"' EXIT INT TERM

if ! TMPDIR="$runtime_tmp" SOAK_SECONDS=36 SOAK_SAMPLE_INTERVAL_SECONDS=10 \
    SOAK_RESTART_INTERVAL_SECONDS=08 SOAK_REPORT_DIR="$report_dir" \
    timeout --signal=TERM --kill-after=5s 90s \
        bash "$project_dir/scripts/soak-daemon.sh" >"$test_root/soak.log" 2>&1; then
    cat "$test_root/soak.log" >&2
    exit 1
fi

grep -Fx 'result=passed' "$report_dir/summary.txt" >/dev/null
if ! grep -Eq '^daemon_restarts=[2-9][0-9]*$' "$report_dir/summary.txt"; then
    cat "$report_dir/summary.txt" >&2
    cat "$test_root/soak.log" >&2
    printf '%s\n' 'soak did not complete the expected daemon restarts' >&2
    exit 1
fi
test -f "$report_dir/daemon.log"
test -s "$report_dir/resource-samples.csv"
if find "$runtime_tmp" -mindepth 1 -maxdepth 1 -type d -name 'wayexpand-soak.*' -print -quit | grep -q .; then
    printf '%s\n' 'restart soak left its private runtime directory behind' >&2
    exit 1
fi

if TMPDIR="$runtime_tmp" SOAK_SECONDS=10 SOAK_RESTART_INTERVAL_SECONDS=259201 \
    bash "$project_dir/scripts/soak-daemon.sh" >"$test_root/invalid.log" 2>&1; then
    printf '%s\n' 'soak accepted an out-of-range restart interval' >&2
    exit 1
fi
grep -F 'SOAK_RESTART_INTERVAL_SECONDS must be 0 or at most 259200' \
    "$test_root/invalid.log" >/dev/null

printf '%s\n' 'daemon restart soak passed'
