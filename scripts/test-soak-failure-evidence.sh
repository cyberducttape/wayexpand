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
private_runtime=$(find "$runtime_tmp" -mindepth 1 -maxdepth 1 -type d -name 'wayexpand-soak.*' -print -quit)
daemon_pid=
for command_path in /proc/[0-9]*/cmdline; do
    candidate_pid=${command_path#/proc/}
    candidate_pid=${candidate_pid%/cmdline}
    candidate_command=$(tr '\0' ' ' 2>/dev/null <"$command_path" || true)
    case "$candidate_command" in
        "$project_dir/target/debug/wayexpand-daemon --source=stdin --backend=none "*) ;;
        *) continue ;;
    esac
    if tr '\0' '\n' 2>/dev/null <"/proc/$candidate_pid/environ" \
        | grep -Fx "XDG_RUNTIME_DIR=$private_runtime" >/dev/null 2>&1; then
        daemon_pid=$candidate_pid
        break
    fi
done
[ -n "$daemon_pid" ] || {
    printf '%s\n' 'could not identify the soak daemon in its private runtime' >&2
    exit 1
}
kill -TERM "$daemon_pid"
set +e
wait "$soak_pid"
soak_status=$?
set -e
soak_pid=
[ "$soak_status" -eq 1 ] || {
    printf 'expected failed soak exit 1 after daemon termination, got %s\n' "$soak_status" >&2
    cat "$test_root/soak.log" >&2
    exit 1
}

grep -Fx 'result=failed' "$report_dir/summary.txt" >/dev/null
grep -Fx 'exit_status=1' "$report_dir/summary.txt" >/dev/null
grep -Fx 'daemon_restarts=0' "$report_dir/summary.txt" >/dev/null
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
