#!/bin/sh
# Long-running, compositor-independent daemon soak. It records process resource
# samples and control-path latency; it does not simulate compositor/device
# lifecycle events. Use SOAK_SECONDS=86400 or 259200 for release soaks.
set -eu
umask 077

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
soak_seconds=${SOAK_SECONDS:-60}
warmup_seconds=$(( soak_seconds / 6 ))
[ "$warmup_seconds" -ge 5 ] || warmup_seconds=5
# Allowed growth after warmup: memory in KiB, descriptors in count.
rss_slack_kib=${SOAK_RSS_SLACK_KIB:-8192}
fd_slack=${SOAK_FD_SLACK:-4}
sample_interval=${SOAK_SAMPLE_INTERVAL_SECONDS:-60}
feed_interval=${SOAK_FEED_INTERVAL_SECONDS:-1}
case "$soak_seconds:$sample_interval:$rss_slack_kib:$fd_slack:$feed_interval" in
    *[!0-9:]*|:*|*::*|*:) printf '%s\n' 'error: duration, feed/sample intervals, and slack values must be non-negative integers' >&2; exit 2 ;;
esac
[ "$soak_seconds" -gt 0 ] && [ "$sample_interval" -gt 0 ] || {
    printf '%s\n' 'error: SOAK_SECONDS and SOAK_SAMPLE_INTERVAL_SECONDS must be positive' >&2
    exit 2
}

runtime_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak.XXXXXX")
config_path="$runtime_dir/expansions.toml"
metrics_path="$runtime_dir/metrics.csv"
status_latency_path="$runtime_dir/status-latency-ms"
explain_latency_path="$runtime_dir/explain-latency-ms"
cli_stderr_path="$runtime_dir/cli.stderr"
report_dir=${SOAK_REPORT_DIR:-}
daemon_pid=
feeder_pid=
if [ -n "$report_dir" ] && [ -e "$report_dir" ]; then
    printf '%s\n' "error: soak report directory already exists: $report_dir" >&2
    exit 2
fi
cleanup() {
    for pid in "$feeder_pid" "$daemon_pid"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$runtime_dir"
}
trap cleanup EXIT INT TERM

write_config() {
    printf '[[expansion]]\ntrigger = ";;sig"\naliases = [";;signature"]\nreplacement = "Best regards %s"\n\n[[expansion]]\ntrigger = ";;date"\nreplacement = "{{date}} {{time}}"\nmatch_mode = "word-boundary"\n' "$1" >"$config_path.tmp"
    chmod 0600 "$config_path.tmp"
    mv "$config_path.tmp" "$config_path"
}
write_config 0

cargo build --locked -q -p wayexpand-daemon -p wayexpand
daemon="$project_dir/target/debug/wayexpand-daemon"
cli="$project_dir/target/debug/wayexpand"

# The feeder types continuously; the daemon reads it as its input stream.
fifo="$runtime_dir/input"
mkfifo "$fifo"
XDG_RUNTIME_DIR="$runtime_dir" WAYEXPAND_CONFIG="$config_path" RUST_LOG=warn \
    "$daemon" --source=stdin --backend=none <"$fifo" >"$runtime_dir/daemon.log" 2>&1 &
daemon_pid=$!
(
    while :; do
        printf 'hello ;;sig and ;;signature then ;;date \n'
        printf 'ordinary words without triggers\n'
        sleep "$feed_interval"
    done
) >"$fifo" &
feeder_pid=$!

ready=0
for _attempt in $(seq 50); do
    if XDG_RUNTIME_DIR="$runtime_dir" "$cli" status >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.1
done
if [ "$ready" -ne 1 ]; then
    printf '%s\n' 'error: daemon did not become ready before the soak' >&2
    cat "$runtime_dir/daemon.log" >&2
    exit 1
fi

sample() {
    elapsed=$1
    rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$daemon_pid/status")
    threads=$(awk '/^Threads:/ {print $2}' "/proc/$daemon_pid/status")
    fds=$(find "/proc/$daemon_pid/fd" -mindepth 1 -maxdepth 1 | wc -l)
    cpu_ticks=$(sed 's/^.*) //' "/proc/$daemon_pid/stat" | awk '{print $12 + $13}')
    wall_ns=$(date +%s%N)
    cpu_percent=$(awk -v ticks="$cpu_ticks" -v previous="$previous_cpu_ticks" \
        -v wall="$wall_ns" -v previous_wall="$previous_sample_wall_ns" \
        -v hz="$ticks_per_second" 'BEGIN {
            if (wall <= previous_wall) { print "0.0"; exit }
            printf "%.1f", (ticks - previous) / hz * 100 / ((wall - previous_wall) / 1000000000)
        }')
    previous_cpu_ticks=$cpu_ticks
    previous_sample_wall_ns=$wall_ns
    timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    printf '%s,%s,%s,%s,%s,%s,%s\n' \
        "$timestamp" "$elapsed" "$round" "$rss" "$threads" "$fds" "$cpu_percent" \
        >>"$metrics_path"
    if [ -z "$baseline" ] && [ "$elapsed" -ge "$warmup_seconds" ]; then
        baseline="$rss $fds"
    fi
}

measure_latency_ms() {
    started_ns=$(date +%s%N)
    if ! "$@" >/dev/null 2>>"$cli_stderr_path"; then
        printf 'error: soak control command failed: %s\n' "$*" >&2
        return 1
    fi
    finished_ns=$(date +%s%N)
    awk -v start="$started_ns" -v finish="$finished_ns" \
        'BEGIN { printf "%.3f", (finish - start) / 1000000 }'
}

percentile() {
    percentile_value=$1
    percentile_file=$2
    sort -n "$percentile_file" | awk -v percentile="$percentile_value" '
        { values[NR] = $1 }
        END {
            if (NR == 0) { print "n/a"; exit }
            rank = int((NR * percentile + 99) / 100)
            if (rank < 1) rank = 1
            print values[rank]
        }'
}

ticks_per_second=$(getconf CLK_TCK)
previous_cpu_ticks=$(sed 's/^.*) //' "/proc/$daemon_pid/stat" | awk '{print $12 + $13}')
previous_sample_wall_ns=$(date +%s%N)
printf '%s\n' 'timestamp_utc,elapsed_seconds,round,rss_kib,threads,open_fds,cpu_percent_since_previous_sample' >"$metrics_path"
: >"$status_latency_path"
: >"$explain_latency_path"
: >"$cli_stderr_path"

start=$(date +%s)
started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
baseline=
round=0
next_sample=$sample_interval
while [ $(( $(date +%s) - start )) -lt "$soak_seconds" ]; do
    kill -0 "$daemon_pid" 2>/dev/null || { echo "daemon exited during soak" >&2; cat "$runtime_dir/daemon.log" >&2; exit 1; }
    round=$((round + 1))
    write_config "$round"
    # Plain status isolates the daemon control round-trip; --json also probes
    # the user's action-broker/systemd state, which is unrelated to this soak.
    status_ms=$(XDG_RUNTIME_DIR="$runtime_dir" measure_latency_ms "$cli" status)
    explain_ms=$(XDG_RUNTIME_DIR="$runtime_dir" measure_latency_ms "$cli" explain ';;sig')
    printf '%s\n' "$status_ms" >>"$status_latency_path"
    printf '%s\n' "$explain_ms" >>"$explain_latency_path"
    if [ $((round % 5)) -eq 0 ]; then
        XDG_RUNTIME_DIR="$runtime_dir" "$cli" pause >/dev/null
        XDG_RUNTIME_DIR="$runtime_dir" "$cli" resume >/dev/null
    fi
    elapsed=$(( $(date +%s) - start ))
    if [ "$elapsed" -ge "$next_sample" ]; then
        sample "$elapsed"
        next_sample=$((next_sample + sample_interval))
    fi
    sleep 0.5
done
elapsed=$(( $(date +%s) - start ))
sample "$elapsed"
[ -n "$baseline" ] || baseline="$rss $fds"

base_rss=${baseline% *}
base_fds=${baseline#* }
final_rss=$rss
final_fds=$fds
echo "soak ${soak_seconds}s, ${round} rounds: rss ${base_rss} -> ${final_rss} KiB, fds ${base_fds} -> ${final_fds}"
echo "final sample: threads=$threads, cpu=${cpu_percent}% since previous sample"
echo "control latency ms: status p50=$(percentile 50 "$status_latency_path") p95=$(percentile 95 "$status_latency_path"); explain p50=$(percentile 50 "$explain_latency_path") p95=$(percentile 95 "$explain_latency_path")"
echo 'resource samples (CSV):'
cat "$metrics_path"
if [ "$final_rss" -gt $((base_rss + rss_slack_kib)) ]; then
    echo "resident memory grew by more than ${rss_slack_kib} KiB after warmup" >&2
    exit 1
fi
if [ "$final_fds" -gt $((base_fds + fd_slack)) ]; then
    echo "open descriptors grew by more than ${fd_slack} after warmup" >&2
    exit 1
fi
if [ -n "$report_dir" ]; then
    report_parent=$(dirname -- "$report_dir")
    mkdir -p "$report_parent"
    if ! mkdir -m 0700 "$report_dir"; then
        echo "refusing to overwrite existing soak report directory: $report_dir" >&2
        exit 2
    fi
    cp "$metrics_path" "$report_dir/resource-samples.csv"
    cp "$status_latency_path" "$report_dir/status-latency-ms.txt"
    cp "$explain_latency_path" "$report_dir/explain-latency-ms.txt"
    cp "$cli_stderr_path" "$report_dir/cli.stderr"
    cp "$runtime_dir/daemon.log" "$report_dir/daemon.log"
    {
        echo "started_utc=$started_utc"
        echo "host=$(hostname)"
        echo "kernel=$(uname -sr)"
        echo "wayexpand_version=$("$cli" --version)"
        echo "daemon_version=$("$daemon" --version)"
        echo "duration_seconds=$soak_seconds"
        echo "warmup_seconds=$warmup_seconds"
        echo "rounds=$round"
        echo "rss_baseline_kib=$base_rss"
        echo "rss_final_kib=$final_rss"
        echo "fd_baseline=$base_fds"
        echo "fd_final=$final_fds"
        echo "threads_final=$threads"
        echo "cpu_percent_last_interval=$cpu_percent"
        echo "status_latency_p50_ms=$(percentile 50 "$status_latency_path")"
        echo "status_latency_p95_ms=$(percentile 95 "$status_latency_path")"
    echo "explain_latency_p50_ms=$(percentile 50 "$explain_latency_path")"
    echo "explain_latency_p95_ms=$(percentile 95 "$explain_latency_path")"
    echo "feed_interval_seconds=$feed_interval"
    } >"$report_dir/summary.txt"
    echo "soak evidence saved to $report_dir"
fi
if [ -s "$cli_stderr_path" ]; then
    echo 'CLI diagnostics during soak:' >&2
    cat "$cli_stderr_path" >&2
fi
echo "daemon soak passed"
