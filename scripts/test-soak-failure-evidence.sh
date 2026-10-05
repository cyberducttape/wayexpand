#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak-failure.XXXXXX")
runtime_tmp="$test_root/runtime"
report_dir="$test_root/report"
mkdir -m 0700 "$runtime_tmp"
soak_pid=
cleanup() {
    if [ -n "$soak_pid" ] && kill -0 "$soak_pid" 2>/dev/null; then
        kill -TERM "$soak_pid" 2>/dev/null || true
        wait "$soak_pid" 2>/dev/null || true
    fi
    rm -rf "$test_root"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

TMPDIR="$runtime_tmp" SOAK_SECONDS=60 SOAK_REPORT_DIR="$report_dir" \
    bash "$project_dir/scripts/soak-daemon.sh" >"$test_root/soak.log" 2>&1 &
soak_pid=$!
attempt=0
while [ "$attempt" -lt 100 ]; do
    if find "$runtime_tmp" -name metrics.csv -type f -print -quit | grep -q .; then
        break
    fi
    if ! kill -0 "$soak_pid" 2>/dev/null; then
        cat "$test_root/soak.log" >&2
        printf '%s\n' 'soak exited before failure preservation could be tested' >&2
        exit 1
    fi
    attempt=$((attempt + 1))
    sleep 0.1
done

if ! find "$runtime_tmp" -name metrics.csv -type f -print -quit | grep -q .; then
    printf '%s\n' 'soak did not reach its sampling loop before the test deadline' >&2
    exit 1
fi
kill -TERM "$soak_pid"
set +e
wait "$soak_pid"
soak_status=$?
set -e
soak_pid=
[ "$soak_status" -eq 143 ] || {
    printf 'expected interrupted soak exit 143, got %s\n' "$soak_status" >&2
    cat "$test_root/soak.log" >&2
    exit 1
}

grep -Fx 'result=interrupted' "$report_dir/summary.txt" >/dev/null
grep -Fx 'exit_status=143' "$report_dir/summary.txt" >/dev/null
grep -Eq '^source_revision=[0-9a-f]{40}$' "$report_dir/summary.txt"
grep -Eq '^source_worktree=(clean|dirty)$' "$report_dir/summary.txt"
test -s "$report_dir/daemon.log"
test -s "$report_dir/resource-samples.csv"
test -s "$report_dir/status-latency-ms.txt"
test -s "$report_dir/explain-latency-ms.txt"
if find "$runtime_tmp" -mindepth 1 -maxdepth 1 -type d -name 'wayexpand-soak.*' -print -quit | grep -q .; then
    printf '%s\n' 'interrupted soak left its private runtime directory behind' >&2
    exit 1
fi

printf '%s\n' 'daemon soak failure evidence test passed'
