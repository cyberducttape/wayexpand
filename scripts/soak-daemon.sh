#!/bin/sh
# Long-running daemon soak: drive the stdin daemon with continuous typing,
# configuration reloads, and control requests for SOAK_SECONDS (default 60),
# then fail if resident memory or open descriptors kept growing after warmup.
# Use SOAK_SECONDS=86400 or 259200 for the 24 h / 72 h release soaks.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
soak_seconds=${SOAK_SECONDS:-60}
warmup_seconds=$(( soak_seconds / 6 ))
[ "$warmup_seconds" -ge 5 ] || warmup_seconds=5
# Allowed growth after warmup: memory in KiB, descriptors in count.
rss_slack_kib=${SOAK_RSS_SLACK_KIB:-8192}
fd_slack=${SOAK_FD_SLACK:-4}

runtime_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak.XXXXXX")
config_path="$runtime_dir/expansions.toml"
daemon_pid=
feeder_pid=
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
XDG_RUNTIME_DIR="$runtime_dir" WAYEXPAND_CONFIG="$config_path" \
    "$daemon" --source=stdin --backend=none <"$fifo" >"$runtime_dir/daemon.log" 2>&1 &
daemon_pid=$!
(
    while :; do
        printf 'hello ;;sig and ;;signature then ;;date \n'
        printf 'ordinary words without triggers\n'
    done
) >"$fifo" &
feeder_pid=$!

for _attempt in $(seq 50); do
    XDG_RUNTIME_DIR="$runtime_dir" "$cli" status >/dev/null 2>&1 && break
    sleep 0.1
done

sample() {
    rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$daemon_pid/status")
    fds=$(find "/proc/$daemon_pid/fd" -mindepth 1 -maxdepth 1 | wc -l)
    printf '%s %s\n' "$rss" "$fds"
}

start=$(date +%s)
baseline=
round=0
while [ $(( $(date +%s) - start )) -lt "$soak_seconds" ]; do
    kill -0 "$daemon_pid" 2>/dev/null || { echo "daemon exited during soak" >&2; cat "$runtime_dir/daemon.log" >&2; exit 1; }
    round=$((round + 1))
    write_config "$round"
    XDG_RUNTIME_DIR="$runtime_dir" "$cli" status >/dev/null
    XDG_RUNTIME_DIR="$runtime_dir" "$cli" explain ';;sig' >/dev/null
    if [ $((round % 5)) -eq 0 ]; then
        XDG_RUNTIME_DIR="$runtime_dir" "$cli" pause >/dev/null
        XDG_RUNTIME_DIR="$runtime_dir" "$cli" resume >/dev/null
    fi
    if [ -z "$baseline" ] && [ $(( $(date +%s) - start )) -ge "$warmup_seconds" ]; then
        baseline=$(sample)
    fi
    sleep 0.5
done
final=$(sample)
[ -n "$baseline" ] || baseline=$final

base_rss=${baseline% *}
base_fds=${baseline#* }
final_rss=${final% *}
final_fds=${final#* }
echo "soak ${soak_seconds}s, ${round} rounds: rss ${base_rss} -> ${final_rss} KiB, fds ${base_fds} -> ${final_fds}"
if [ "$final_rss" -gt $((base_rss + rss_slack_kib)) ]; then
    echo "resident memory grew by more than ${rss_slack_kib} KiB after warmup" >&2
    exit 1
fi
if [ "$final_fds" -gt $((base_fds + fd_slack)) ]; then
    echo "open descriptors grew by more than ${fd_slack} after warmup" >&2
    exit 1
fi
echo "daemon soak passed"
