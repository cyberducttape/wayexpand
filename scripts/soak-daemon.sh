#!/bin/sh
# Long-running, compositor-independent daemon soak. It records process resource
# samples and control-path latency, exercises configuration reload/pause cycles,
# and restarts the daemon at a bounded interval. It does not simulate
# compositor/device lifecycle events. Use SOAK_SECONDS=86400 or 259200 for
# release soaks.
set -eu
umask 077

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
source_revision=$(git -C "$project_dir" rev-parse HEAD)
source_worktree=$(sh "$project_dir/scripts/source-tree-state.sh" "$project_dir" \
    Cargo.toml Cargo.lock .cargo rust-toolchain.toml crates scripts tests/certification)
soak_seconds=${SOAK_SECONDS:-60}
warmup_seconds=$(( soak_seconds / 6 ))
[ "$warmup_seconds" -ge 5 ] || warmup_seconds=5
# Allowed peak growth after warmup: memory in KiB, descriptors and threads in count.
rss_slack_kib=${SOAK_RSS_SLACK_KIB:-8192}
fd_slack=${SOAK_FD_SLACK:-4}
# A small amount of transient worker variation is allowed; sustained thread
# growth beyond this limit fails the soak just like descriptor growth.
thread_slack=${SOAK_THREAD_SLACK:-2}
sample_interval=${SOAK_SAMPLE_INTERVAL_SECONDS:-60}
feed_interval=${SOAK_FEED_INTERVAL_SECONDS:-1}
restart_interval=${SOAK_RESTART_INTERVAL_SECONDS:-21600}
case "$soak_seconds:$sample_interval:$rss_slack_kib:$fd_slack:$thread_slack:$feed_interval:$restart_interval" in
    *[!0-9:]*|:*|*::*|*:) printf '%s\n' 'error: duration, feed/sample intervals, and slack values must be non-negative integers' >&2; exit 2 ;;
esac
if [ "$soak_seconds" -le 0 ] || [ "$sample_interval" -le 0 ]; then
    printf '%s\n' 'error: SOAK_SECONDS and SOAK_SAMPLE_INTERVAL_SECONDS must be positive' >&2
    exit 2
fi
normalized_restart_interval=$(awk -v interval="$restart_interval" '
    BEGIN {
        if (interval == 0) print 0
        else if (interval > 0 && interval <= 259200) printf "%.0f\n", interval
        else exit 1
    }
') || {
    printf '%s\n' 'error: SOAK_RESTART_INTERVAL_SECONDS must be 0 or at most 259200' >&2
    exit 2
}
restart_interval=$normalized_restart_interval

runtime_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak.XXXXXX")
config_path="$runtime_dir/expansions.toml"
metrics_path="$runtime_dir/metrics.csv"
status_latency_path="$runtime_dir/status-latency-ms"
explain_latency_path="$runtime_dir/explain-latency-ms"
restart_latency_path="$runtime_dir/restart-latency-ms"
cli_stderr_path="$runtime_dir/cli.stderr"
report_dir=${SOAK_REPORT_DIR:-}
daemon_pid=
feeder_pid=
report_created=0
preserve_partial_report() {
    failure_status=$1
    [ -n "$report_dir" ] || return 0
    if [ "$report_created" -eq 0 ]; then
        mkdir -p "$(dirname -- "$report_dir")" || return 0
        mkdir -m 0700 "$report_dir" || return 0
        report_created=1
    fi
    copy_partial_artifact() {
        if [ -f "$1" ]; then
            cp "$1" "$report_dir/$2" || true
        fi
    }
    copy_partial_artifact "$metrics_path" resource-samples.csv
    copy_partial_artifact "$status_latency_path" status-latency-ms.txt
    copy_partial_artifact "$explain_latency_path" explain-latency-ms.txt
    copy_partial_artifact "$restart_latency_path" daemon-restart-latency-ms.txt
    copy_partial_artifact "$cli_stderr_path" cli.stderr
    copy_partial_artifact "$runtime_dir/daemon.log" daemon.log
    elapsed_seconds=0
    if [ -n "${start:-}" ]; then
        elapsed_seconds=$(( $(date +%s) - start ))
    fi
    if [ "$failure_status" -eq 130 ] || [ "$failure_status" -eq 143 ]; then
        failure_result=interrupted
    else
        failure_result=failed
    fi
    {
        echo "result=$failure_result"
        echo "exit_status=$failure_status"
        echo "started_utc=${started_utc:-unknown}"
        echo "finished_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "kernel=$(uname -sr)"
        echo "source_revision=$source_revision"
        echo "source_worktree=$source_worktree"
        echo "duration_seconds=$soak_seconds"
        echo "elapsed_seconds=$elapsed_seconds"
        echo "rounds=${round:-0}"
        echo "daemon_restarts=${daemon_restarts:-0}"
        echo "failure_reason=${failure_reason:-not-recorded}"
        echo "rss_slack_kib=$rss_slack_kib"
        echo "fd_slack=$fd_slack"
        echo "rss_peak_after_warmup_kib=${rss_peak_after_warmup:-not-sampled}"
        echo "fds_peak_after_warmup=${fds_peak_after_warmup:-not-sampled}"
        echo "rss_baseline_kib=${base_rss:-not-sampled}"
        echo "fd_baseline=${base_fds:-not-sampled}"
        echo "thread_slack=$thread_slack"
        echo "threads_baseline=${base_threads:-not-sampled}"
        echo "threads_peak_after_warmup=${threads_peak_after_warmup:-not-sampled}"
        echo "threads_final=${threads:-not-sampled}"
    } >"$report_dir/summary.txt" || true
}

private_process_pids() {
    for environment in /proc/[0-9]*/environ; do
        [ -r "$environment" ] || continue
        pid=${environment#/proc/}
        pid=${pid%/environ}
        if cat "$environment" 2>/dev/null | tr '\0' '\n' \
            | grep -Fx "XDG_RUNTIME_DIR=$runtime_dir" >/dev/null 2>&1; then
            printf '%s\n' "$pid"
        fi
    done
}

reap_private_processes() {
    # A background shell function can leave its env-launched daemon orphaned
    # when the wrapper PID is killed. Match the randomized private runtime
    # inherited by the daemon and any of its descendants, then reap the whole
    # isolated process set before removing that runtime directory.
    private_pids=$(private_process_pids)
    for pid in $private_pids; do
        kill -TERM "$pid" 2>/dev/null || true
    done
    attempt=0
    while [ "$attempt" -lt 20 ]; do
        private_pids=$(private_process_pids)
        [ -n "$private_pids" ] || break
        attempt=$((attempt + 1))
        sleep 0.05
    done
    for pid in $(private_process_pids); do
        kill -KILL "$pid" 2>/dev/null || true
    done
}

cleanup() {
    cleanup_status=$?
    trap - EXIT INT TERM
    for pid in "$feeder_pid" "$daemon_pid"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    reap_private_processes
    for pid in "$feeder_pid" "$daemon_pid"; do
        if [ -n "$pid" ]; then
            wait "$pid" 2>/dev/null || true
        fi
    done
    if [ "$cleanup_status" -ne 0 ]; then
        preserve_partial_report "$cleanup_status"
    fi
    rm -rf "$runtime_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
if [ -n "$report_dir" ] && [ -e "$report_dir" ]; then
    printf '%s\n' "error: soak report directory already exists: $report_dir" >&2
    exit 2
fi

# Keep the synthetic daemon out of the developer's desktop session. A private
# XDG_RUNTIME_DIR alone is insufficient when D-Bus, Wayland, or X11 addresses
# are inherited: on KDE, the daemon can otherwise install a real KWin tracker
# script while the soak claims to be compositor-independent.
mkdir -m 0700 "$runtime_dir/home"
. "$project_dir/scripts/soak-isolation.sh"

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
daemon_restarts=0

start_daemon() {
    env -i \
        PATH="$PATH" \
        HOME="$runtime_dir/home" \
        TMPDIR="$runtime_dir" \
        XDG_RUNTIME_DIR="$runtime_dir" \
        WAYEXPAND_CONFIG="$config_path" \
        RUST_LOG=warn \
        "$daemon" --source=stdin --backend=none \
        <"$fifo" >>"$runtime_dir/daemon.log" 2>&1 &
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
        if ! kill -0 "$daemon_pid" 2>/dev/null; then
            break
        fi
        if run_isolated "$runtime_dir" "$config_path" "$cli" status >/dev/null 2>&1; then
            ready=1
            break
        fi
        sleep 0.1
    done
    if [ "$ready" -ne 1 ]; then
        printf '%s\n' 'error: daemon did not become ready before the soak' >&2
        cat "$runtime_dir/daemon.log" >&2
        return 1
    fi
    daemon_executable=$(readlink "/proc/$daemon_pid/exe" 2>/dev/null || true)
    case "$daemon_executable" in
        */wayexpand-daemon) ;;
        *)
            printf '%s\n' 'error: soak resource tracking did not attach to the daemon process' >&2
            return 1
            ;;
    esac
}

restart_daemon() {
    restart_started_ns=$(date +%s%N)
    if [ -n "$feeder_pid" ] && kill -0 "$feeder_pid" 2>/dev/null; then
        kill "$feeder_pid" 2>/dev/null || true
        wait "$feeder_pid" 2>/dev/null || true
    fi
    feeder_pid=
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        # SIGTERM bypasses Rust Drop and leaves the control socket behind.
        # The next daemon can then be mistaken for the old one or fail to bind.
        run_isolated "$runtime_dir" "$config_path" "$cli" stop >/dev/null
        stopped=0
        for _attempt in $(seq 50); do
            if ! kill -0 "$daemon_pid" 2>/dev/null; then
                stopped=1
                break
            fi
            sleep 0.1
        done
        if [ "$stopped" -ne 1 ]; then
            printf '%s\n' 'error: daemon did not stop cleanly through its control socket' >&2
            return 1
        fi
        if ! wait "$daemon_pid"; then
            printf '%s\n' 'error: daemon failed while stopping through its control socket' >&2
            return 1
        fi
    fi
    daemon_pid=
    start_daemon
    restart_finished_ns=$(date +%s%N)
    restart_latency_ms=$(awk -v started="$restart_started_ns" -v finished="$restart_finished_ns" \
        'BEGIN { printf "%.3f", (finished - started) / 1000000 }')
    printf '%s\n' "$restart_latency_ms" >>"$restart_latency_path"
    daemon_restarts=$((daemon_restarts + 1))
    # The new process has a fresh CPU-tick origin; reset the interval baseline
    # so resource evidence never reports a negative or cross-process CPU delta.
    previous_cpu_ticks=$(sed 's/^.*) //' "/proc/$daemon_pid/stat" | awk '{print $12 + $13}')
    previous_sample_wall_ns=$(date +%s%N)
}

start_daemon

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
    printf '%s,%s,%s,%s,%s,%s,%s,%s\n' \
        "$timestamp" "$elapsed" "$round" "$daemon_pid" "$rss" "$threads" "$fds" "$cpu_percent" \
        >>"$metrics_path"
    if [ -z "$baseline" ] && [ "$elapsed" -ge "$warmup_seconds" ]; then
        baseline="$rss $fds $threads"
        rss_peak_after_warmup=$rss
        fds_peak_after_warmup=$fds
        threads_peak_after_warmup=$threads
    elif [ -n "$baseline" ]; then
        [ "$rss" -le "$rss_peak_after_warmup" ] || rss_peak_after_warmup=$rss
        [ "$fds" -le "$fds_peak_after_warmup" ] || fds_peak_after_warmup=$fds
        [ "$threads" -le "$threads_peak_after_warmup" ] || threads_peak_after_warmup=$threads
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
printf '%s\n' 'timestamp_utc,elapsed_seconds,round,daemon_pid,rss_kib,threads,open_fds,cpu_percent_since_previous_sample' >"$metrics_path"
: >"$status_latency_path"
: >"$explain_latency_path"
: >"$restart_latency_path"
: >"$cli_stderr_path"

start=$(date +%s)
started_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
next_restart=$((start + restart_interval))
baseline=
rss_peak_after_warmup=0
fds_peak_after_warmup=0
threads_peak_after_warmup=0
round=0
next_sample=$warmup_seconds
while [ $(( $(date +%s) - start )) -lt "$soak_seconds" ]; do
    kill -0 "$daemon_pid" 2>/dev/null || { echo "daemon exited during soak" >&2; cat "$runtime_dir/daemon.log" >&2; exit 1; }
    round=$((round + 1))
    write_config "$round"
    # Plain status isolates the daemon control round-trip; --json also probes
    # the user's action-broker/systemd state, which is unrelated to this soak.
    if ! status_ms=$(measure_latency_ms run_isolated "$runtime_dir" "$config_path" "$cli" status); then
        printf '%s\n' 'error: daemon status probe failed during the soak' >&2
        exit 1
    fi
    if ! explain_ms=$(measure_latency_ms run_isolated "$runtime_dir" "$config_path" "$cli" explain ';;sig'); then
        printf '%s\n' 'error: daemon explain probe failed during the soak' >&2
        exit 1
    fi
    printf '%s\n' "$status_ms" >>"$status_latency_path"
    printf '%s\n' "$explain_ms" >>"$explain_latency_path"
    if [ $((round % 5)) -eq 0 ]; then
        run_isolated "$runtime_dir" "$config_path" "$cli" pause >/dev/null
        run_isolated "$runtime_dir" "$config_path" "$cli" resume >/dev/null
    fi
    elapsed=$(( $(date +%s) - start ))
    if [ "$restart_interval" -gt 0 ] && [ "$(date +%s)" -ge "$next_restart" ]; then
        restart_daemon
        next_restart=$(( $(date +%s) + restart_interval ))
    fi
    if [ "$elapsed" -ge "$next_sample" ]; then
        sample "$elapsed"
        next_sample=$((elapsed + sample_interval))
    fi
    sleep 0.5
done
elapsed=$(( $(date +%s) - start ))
sample "$elapsed"
[ -n "$baseline" ] || {
    baseline="$rss $fds $threads"
    rss_peak_after_warmup=$rss
    fds_peak_after_warmup=$fds
    threads_peak_after_warmup=$threads
}

base_rss=$(printf '%s\n' "$baseline" | awk '{print $1}')
base_fds=$(printf '%s\n' "$baseline" | awk '{print $2}')
base_threads=$(printf '%s\n' "$baseline" | awk '{print $3}')
final_rss=$rss
final_fds=$fds
final_threads=$threads
echo "soak ${soak_seconds}s, ${round} rounds: rss ${base_rss} -> ${final_rss} KiB (peak ${rss_peak_after_warmup}), fds ${base_fds} -> ${final_fds} (peak ${fds_peak_after_warmup}), threads ${base_threads} -> ${final_threads} (peak ${threads_peak_after_warmup})"
echo "final sample: cpu=${cpu_percent}% since previous sample"
echo "control latency ms: status p50=$(percentile 50 "$status_latency_path") p95=$(percentile 95 "$status_latency_path"); explain p50=$(percentile 50 "$explain_latency_path") p95=$(percentile 95 "$explain_latency_path")"
echo 'resource samples (CSV):'
cat "$metrics_path"
if [ "$rss_peak_after_warmup" -gt $((base_rss + rss_slack_kib)) ]; then
    failure_reason="resident memory grew by more than ${rss_slack_kib} KiB after warmup"
    echo "$failure_reason" >&2
    exit 1
fi
if [ "$fds_peak_after_warmup" -gt $((base_fds + fd_slack)) ]; then
    failure_reason="open descriptors grew by more than ${fd_slack} after warmup"
    echo "$failure_reason" >&2
    exit 1
fi
if [ "$threads_peak_after_warmup" -gt $((base_threads + thread_slack)) ]; then
    failure_reason="threads grew by more than ${thread_slack} after warmup (peak ${threads_peak_after_warmup}, baseline ${base_threads})"
    echo "$failure_reason" >&2
    exit 1
fi
if [ -n "$report_dir" ]; then
    report_parent=$(dirname -- "$report_dir")
    mkdir -p "$report_parent"
    if ! mkdir -m 0700 "$report_dir"; then
        echo "refusing to overwrite existing soak report directory: $report_dir" >&2
        exit 2
    fi
    report_created=1
    cp "$metrics_path" "$report_dir/resource-samples.csv"
    cp "$status_latency_path" "$report_dir/status-latency-ms.txt"
    cp "$explain_latency_path" "$report_dir/explain-latency-ms.txt"
    cp "$restart_latency_path" "$report_dir/daemon-restart-latency-ms.txt"
    cp "$cli_stderr_path" "$report_dir/cli.stderr"
    cp "$runtime_dir/daemon.log" "$report_dir/daemon.log"
    {
        echo "result=passed"
        echo "started_utc=$started_utc"
        echo "finished_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "kernel=$(uname -sr)"
        echo "source_revision=$source_revision"
        echo "source_worktree=$source_worktree"
        echo "wayexpand_version=$("$cli" --version)"
        echo "daemon_version=$("$daemon" --version)"
        echo "sampled_executable=$daemon_executable"
        echo "sampled_daemon_pid=$daemon_pid"
        echo "duration_seconds=$soak_seconds"
        echo "warmup_seconds=$warmup_seconds"
        echo "rounds=$round"
        echo "daemon_restarts=$daemon_restarts"
        echo "rss_baseline_kib=$base_rss"
        echo "rss_final_kib=$final_rss"
        echo "rss_slack_kib=$rss_slack_kib"
        echo "rss_peak_after_warmup_kib=$rss_peak_after_warmup"
        echo "fd_baseline=$base_fds"
        echo "fd_final=$final_fds"
        echo "fd_peak_after_warmup=$fds_peak_after_warmup"
        echo "fd_slack=$fd_slack"
        echo "threads_baseline=$base_threads"
        echo "threads_peak_after_warmup=$threads_peak_after_warmup"
        echo "threads_final=$final_threads"
        echo "thread_slack=$thread_slack"
        echo "cpu_percent_last_interval=$cpu_percent"
        echo "status_latency_p50_ms=$(percentile 50 "$status_latency_path")"
        echo "status_latency_p95_ms=$(percentile 95 "$status_latency_path")"
        echo "explain_latency_p50_ms=$(percentile 50 "$explain_latency_path")"
        echo "explain_latency_p95_ms=$(percentile 95 "$explain_latency_path")"
        echo "restart_latency_p50_ms=$(percentile 50 "$restart_latency_path")"
        echo "restart_latency_p95_ms=$(percentile 95 "$restart_latency_path")"
        echo "feed_interval_seconds=$feed_interval"
    } >"$report_dir/summary.txt"
    echo "soak evidence saved to $report_dir"
fi
if [ -s "$cli_stderr_path" ]; then
    echo 'CLI diagnostics during soak:' >&2
    cat "$cli_stderr_path" >&2
fi
echo "daemon soak passed"
