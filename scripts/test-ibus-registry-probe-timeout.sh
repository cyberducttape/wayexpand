#!/bin/sh
# Prove that an unresponsive but accepting IBus socket cannot make a registry
# probe exceed its per-call timeout.
set -eu

for tool in ibus python3 timeout; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'error: %s is required for the IBus registry timeout test\n' "$tool" >&2
        exit 2
    fi
done

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-ibus-stall.XXXXXX")
socket="$work/ibus.sock"
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT INT TERM

# This accepts the connection but never sends the D-Bus authentication
# greeting or a method reply. The real IBus client therefore blocks in the
# exact class of call guarded by the registry probe's timeout.
python3 - "$socket" <<'PY' &
import socket
import sys
import time

listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
listener.bind(sys.argv[1])
listener.listen(1)
connection, _ = listener.accept()
time.sleep(10)
connection.close()
listener.close()
PY
server_pid=$!

tries=100
while [ ! -S "$socket" ] && [ "$tries" -gt 0 ]; do
    sleep 0.01
    tries=$((tries - 1))
done
if [ ! -S "$socket" ]; then
    printf '%s\n' "fail stalled IBus socket was not created" >&2
    exit 1
fi

started_ns=$(date +%s%N)
probe_status=0
IBUS_ADDRESS="unix:path=$socket" timeout --signal=TERM --kill-after=0.2s 0.5s \
    ibus list-engine --name-only >/dev/null 2>&1 || probe_status=$?
finished_ns=$(date +%s%N)
elapsed_ms=$(((finished_ns - started_ns) / 1000000))

if [ "$probe_status" -ne 124 ] && [ "$probe_status" -ne 137 ]; then
    printf 'fail stalled IBus registry probe exited with %s, expected timeout (elapsed %s ms)\n' \
        "$probe_status" "$elapsed_ms" >&2
    exit 1
fi
if [ "$elapsed_ms" -lt 400 ] || [ "$elapsed_ms" -ge 2000 ]; then
    printf 'fail stalled IBus registry probe took %s ms, expected 400–1999 ms\n' \
        "$elapsed_ms" >&2
    exit 1
fi

printf 'pass stalled IBus registry probe timed out in %s ms (status %s)\n' \
    "$elapsed_ms" "$probe_status"
