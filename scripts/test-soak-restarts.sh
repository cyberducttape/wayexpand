#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak-restarts.XXXXXX")
runtime_tmp="$test_root/runtime"
report_dir="$test_root/report"
mkdir -m 0700 "$runtime_tmp"
trap 'rm -rf "$test_root"' EXIT INT TERM

if ! TMPDIR="$runtime_tmp" SOAK_SECONDS=36 SOAK_SAMPLE_INTERVAL_SECONDS=10 \
    SOAK_THREAD_SLACK=0 SOAK_RESTART_INTERVAL_SECONDS=08 SOAK_REPORT_DIR="$report_dir" \
    timeout --signal=TERM --kill-after=5s 90s \
        bash "$project_dir/scripts/soak-daemon.sh" >"$test_root/soak.log" 2>&1; then
    cat "$test_root/soak.log" >&2
    exit 1
fi

grep -Fx 'result=passed' "$report_dir/summary.txt" >/dev/null
if grep -q '^host=' "$report_dir/summary.txt"; then
    printf '%s\n' 'soak evidence exposed the host name' >&2
    exit 1
fi
if ! grep -Eq '^daemon_restarts=[2-9][0-9]*$' "$report_dir/summary.txt"; then
    cat "$report_dir/summary.txt" >&2
    cat "$test_root/soak.log" >&2
    printf '%s\n' 'soak did not complete the expected daemon restarts' >&2
    exit 1
fi
test -f "$report_dir/daemon.log"
test -s "$report_dir/resource-samples.csv"
grep -Fx 'timestamp_utc,elapsed_seconds,round,daemon_pid,rss_kib,threads,open_fds,cpu_percent_since_previous_sample' \
    "$report_dir/resource-samples.csv" >/dev/null
awk -F, 'NR > 1 && $4 !~ /^[0-9]+$/ { exit 1 } END { if (NR < 2) exit 1 }' \
    "$report_dir/resource-samples.csv"
grep -Eq '^sampled_executable=.*/wayexpand-daemon$' "$report_dir/summary.txt"
restart_count=$(sed -n 's/^daemon_restarts=//p' "$report_dir/summary.txt")
test "$(wc -l <"$report_dir/daemon-restart-latency-ms.txt")" -eq "$restart_count"
grep -Eq '^restart_latency_p50_ms=[0-9]+\.[0-9]{3}$' "$report_dir/summary.txt"
grep -Eq '^restart_latency_p95_ms=[0-9]+\.[0-9]{3}$' "$report_dir/summary.txt"
grep -Eq '^rss_peak_after_warmup_kib=[0-9]+$' "$report_dir/summary.txt"
grep -Eq '^fd_peak_after_warmup=[0-9]+$' "$report_dir/summary.txt"
grep -Eq '^threads_baseline=[0-9]+$' "$report_dir/summary.txt"
grep -Eq '^threads_peak_after_warmup=[0-9]+$' "$report_dir/summary.txt"
grep -Eq '^threads_final=[0-9]+$' "$report_dir/summary.txt"
grep -Fx 'thread_slack=0' "$report_dir/summary.txt" >/dev/null
thread_baseline=$(sed -n 's/^threads_baseline=//p' "$report_dir/summary.txt")
thread_peak=$(sed -n 's/^threads_peak_after_warmup=//p' "$report_dir/summary.txt")
thread_final=$(sed -n 's/^threads_final=//p' "$report_dir/summary.txt")
rss_baseline=$(sed -n 's/^rss_baseline_kib=//p' "$report_dir/summary.txt")
rss_peak=$(sed -n 's/^rss_peak_after_warmup_kib=//p' "$report_dir/summary.txt")
fd_baseline=$(sed -n 's/^fd_baseline=//p' "$report_dir/summary.txt")
fd_peak=$(sed -n 's/^fd_peak_after_warmup=//p' "$report_dir/summary.txt")
[ "$thread_peak" -le "$thread_baseline" ]
[ "$thread_final" -le "$thread_baseline" ]
[ "$rss_peak" -le $((rss_baseline + 8192)) ]
[ "$fd_peak" -le $((fd_baseline + 4)) ]
for environment in /proc/[0-9]*/environ; do
    [ -r "$environment" ] || continue
    runtime_value=$(cat "$environment" 2>/dev/null | tr '\0' '\n' \
        | sed -n 's/^XDG_RUNTIME_DIR=//p' | head -n 1) || continue
    case "$runtime_value" in
        "$runtime_tmp"/wayexpand-soak.*)
            printf '%s\n' 'completed soak left a process running in its private runtime' >&2
            exit 1
            ;;
    esac
done
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

if TMPDIR="$runtime_tmp" SOAK_SECONDS=10 SOAK_THREAD_SLACK=-1 \
    bash "$project_dir/scripts/soak-daemon.sh" >"$test_root/invalid-thread-slack.log" 2>&1; then
    printf '%s\n' 'soak accepted a negative thread-growth allowance' >&2
    exit 1
fi
grep -F 'slack values must be non-negative integers' \
    "$test_root/invalid-thread-slack.log" >/dev/null

printf '%s\n' 'daemon restart soak passed'
