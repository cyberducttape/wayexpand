#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
runtime_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-smoke.XXXXXX")
daemon_pid=
config_path="$runtime_dir/expansions.toml"

cp "$project_dir/expansions.toml" "$config_path"
chmod 0600 "$config_path"

cleanup() {
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf "$runtime_dir"
}
trap cleanup EXIT INT TERM

wait_for_config_state() {
    expected=$1
    status=
    for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        if status=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" status 2>/dev/null) \
            && printf '%s\n' "$status" | grep -F "config_state=$expected" >/dev/null; then
            return 0
        fi
        sleep 0.1
    done
    printf '%s\n' "expected config_state=$expected, got:" "$status" >&2
    return 1
}

wait_for_status_field() {
    field=$1
    expected=$2
    status=
    for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        if status=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" status 2>/dev/null) \
            && printf '%s\n' "$status" | grep -F "${field}=${expected}" >/dev/null; then
            return 0
        fi
        sleep 0.1
    done
    printf '%s\n' "expected ${field}=${expected}, got:" "$status" >&2
    return 1
}

XDG_RUNTIME_DIR="$runtime_dir" \
WAYEXPAND_CONFIG="$config_path" \
"$project_dir/target/debug/wayexpand-daemon" \
    --source=stdin --backend=none \
    </dev/null >"$runtime_dir/daemon.log" 2>&1 &
daemon_pid=$!

status=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    if status=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" status 2>/dev/null); then
        case "$status" in
            *"source=stdin"*"backend=none"*"config_state=ok"*) break ;;
        esac
    else
        status="(no response from status command)"
    fi
    sleep 0.1
done

case "$status" in
    *"source=stdin"*"backend=none"*"config_state=ok"*) ;;
    *)
        printf '%s\n' "unexpected status response:" "$status" >&2
        printf '%s\n' "daemon log:" >&2
        cat "$runtime_dir/daemon.log" >&2 || true
        exit 1
        ;;
esac

[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" pause)" = "paused" ]
wait_for_status_field paused true
[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" resume)" = "resumed" ]
wait_for_status_field paused false

printf '%s\n' '[[expansion]]' 'trigger = ' >"$config_path"
reload=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" reload)
[ "$reload" = "reload scheduled" ]
wait_for_config_state "reload-rejected"

cp "$project_dir/expansions.toml" "$config_path"
reload=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" reload)
[ "$reload" = "reload scheduled" ]
wait_for_config_state "ok"

reload=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" reload)
[ "$reload" = "reload scheduled" ]

stop=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" stop)
[ "$stop" = "stopping" ]
wait "$daemon_pid"
daemon_pid=
[ ! -e "$runtime_dir/wayexpand.sock" ]

XDG_RUNTIME_DIR="$runtime_dir" \
WAYEXPAND_CONFIG="$config_path" \
"$project_dir/target/debug/wayexpand-daemon" \
    </dev/null >"$runtime_dir/daemon-sigterm.log" 2>&1 &
daemon_pid=$!

status=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    if status=$(XDG_RUNTIME_DIR="$runtime_dir" "$project_dir/target/debug/wayexpand" status 2>/dev/null); then
        break
    fi
    sleep 0.1
done
case "$status" in
    *"source=stdin"*"backend=libei"*) ;;
    *)
        printf '%s\n' "daemon did not become ready before SIGTERM test" >&2
        exit 1
        ;;
esac
kill -TERM "$daemon_pid"
(
    sleep 2
    kill -KILL "$daemon_pid" 2>/dev/null || true
) &
watchdog_pid=$!
set +e
wait "$daemon_pid"
daemon_status=$?
set -e
kill "$watchdog_pid" 2>/dev/null || true
wait "$watchdog_pid" 2>/dev/null || true
if [ "$daemon_status" -ne 0 ]; then
    printf '%s\n' "daemon did not exit cleanly after SIGTERM (status $daemon_status)" >&2
    exit 1
fi
daemon_pid=
[ ! -e "$runtime_dir/wayexpand.sock" ]

printf '%s\n' "daemon smoke test passed"
