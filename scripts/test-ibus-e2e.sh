#!/bin/sh
# End-to-end test of the WayExpand IBus engine against a real ibus-daemon.
#
# Starts a private D-Bus session and a private ibus-daemon (own HOME,
# runtime directory, component directory and in-memory GSettings), so it
# never touches the desktop's IBus. A scripted client (wx-e2e-ibus) then
# behaves like a toolkit text field. This exercises the real daemon <->
# engine protocol: component registration, the ContentType property,
# refocus without a new content type, password fields, clients without
# surrounding text, and the engine's content-free activity file.
#
# Usage: scripts/test-ibus-e2e.sh [--keep]
#   E2E_BIN=DIR        use prebuilt wayexpand-ibus and wx-e2e-ibus from DIR
#   E2E_SKIP_BUILD=1   do not run cargo build
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
keep=0
for argument in "$@"; do
    case "$argument" in
        --keep) keep=1 ;;
        *) printf '%s\n' "usage: $0 [--keep]" >&2; exit 2 ;;
    esac
done

for tool in ibus ibus-daemon dbus-run-session timeout date; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf '%s\n' "error: $tool is required (install ibus and dbus-daemon)" >&2
        exit 2
    fi
done

# Exercise the registry-call deadline independently of the private daemon
# startup test using a socket that accepts but never answers D-Bus requests.
sh "$project_dir/scripts/test-ibus-registry-probe-timeout.sh"

bin_dir=${E2E_BIN:-"$project_dir/target/debug"}
if [ "${E2E_SKIP_BUILD:-0}" != 1 ]; then
    cargo build --locked --quiet \
        -p wayexpand-backend-ibus --bin wayexpand-ibus \
        -p wayexpand-e2e --bin wx-e2e-ibus
fi
for binary in wayexpand-ibus wx-e2e-ibus; do
    if [ ! -x "$bin_dir/$binary" ]; then
        printf '%s\n' "error: $bin_dir/$binary is missing" >&2
        exit 2
    fi
done

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-ibus-e2e.XXXXXX")
cleanup() {
    if [ -f "$work/runtime/wayexpand-ibus-status.json" ]; then
        engine_pid=$(sed -n 's/.*"pid":\([0-9][0-9]*\).*/\1/p' "$work/runtime/wayexpand-ibus-status.json")
        if [ -n "$engine_pid" ] && [ "$(cat "/proc/$engine_pid/comm" 2>/dev/null)" = wayexpand-ibus ]; then
            kill "$engine_pid" 2>/dev/null || true
        fi
    fi
    if [ "$keep" -eq 1 ]; then
        printf '%s\n' "kept $work"
    else
        rm -rf -- "$work"
    fi
}
trap cleanup EXIT INT TERM

mkdir -p "$work/home" "$work/runtime" "$work/config/wayexpand" "$work/component" "$work/cache"
chmod 0700 "$work/home" "$work/runtime" "$work/config" "$work/config/wayexpand"
cat >"$work/config/wayexpand/expansions.toml" <<'EOF'
[settings]
compatibility_mode = "tracked-focus"

[[expansion]]
trigger = ";e2e"
replacement = "expanded"
EOF
chmod 0600 "$work/config/wayexpand/expansions.toml"
sed "s|<exec>wayexpand-ibus</exec>|<exec>$bin_dir/wayexpand-ibus</exec>|" \
    "$project_dir/desktop/wayexpand-ibus.xml" >"$work/component/wayexpand.xml"

cat >"$work/session.sh" <<'EOF'
#!/bin/sh
set -eu
socket="$XDG_RUNTIME_DIR/ibus.sock"
ibus-daemon --single --panel=disable --emoji-extension=disable --config=disable \
    --cache=none --address="unix:path=$socket" >"$E2E_WORK/ibus-daemon.log" 2>&1 &
daemon_pid=$!
tries=100
while [ ! -S "$socket" ] && [ "$tries" -gt 0 ]; do
    sleep 0.1
    tries=$((tries - 1))
done
if [ ! -S "$socket" ]; then
    printf '%s\n' "fail daemon ibus-daemon did not create $socket"
    kill "$daemon_pid" 2>/dev/null || true
    exit 1
fi
export IBUS_ADDRESS="unix:path=$socket"
# Socket creation precedes IBus service readiness. Wait for the private
# daemon to answer a registry request and load the WayExpand component before
# asking it to select the engine; otherwise SetGlobalEngine can race startup
# and fail with "IBus connection unavailable".
deadline_ns=$(($(date +%s%N) + 15000000000))
probe_timeouts=0
registered=0
while [ "$(date +%s%N)" -lt "$deadline_ns" ]; do
    probe_status=0
    registry=$(timeout --signal=TERM --kill-after=0.2s 0.5s \
        ibus list-engine --name-only 2>/dev/null) || probe_status=$?
    if [ "$probe_status" -eq 124 ] || [ "$probe_status" -eq 137 ]; then
        probe_timeouts=$((probe_timeouts + 1))
    fi
    if [ "$probe_status" -eq 0 ] &&
        printf '%s\n' "$registry" | grep -Fxq wayexpand; then
        registered=1
        break
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
        printf '%s\n' "fail daemon ibus-daemon exited before registering wayexpand"
        tail -n 40 "$E2E_WORK/ibus-daemon.log" >&2 2>/dev/null || true
        exit 1
    fi
    sleep 0.1
done
if [ "$registered" -ne 1 ]; then
    printf 'fail daemon IBus did not register wayexpand within 15 seconds (%s registry probes timed out)\n' "$probe_timeouts"
    timeout --signal=TERM --kill-after=0.2s 0.5s \
        ibus list-engine --name-only >&2 2>&1 || true
    tail -n 40 "$E2E_WORK/ibus-daemon.log" >&2 2>/dev/null || true
    exit 1
fi
status=0
"$E2E_BIN_DIR/wx-e2e-ibus" "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json" || status=$?
if [ "$status" -eq 0 ]; then
    # Exercise a toolkit joining an already-selected global engine, as GNOME
    # does after changing its input source. This catches tests that only pass
    # because their helper itself first changes IBus's global selection.
    "$E2E_BIN_DIR/wx-e2e-ibus" \
        "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json" \
        --expect-active --single-expansion || status=$?
fi
kill "$daemon_pid" 2>/dev/null || true
wait "$daemon_pid" 2>/dev/null || true
exit "$status"
EOF
chmod 0755 "$work/session.sh"

status=0
env -i \
    HOME="$work/home" \
    PATH="$PATH" \
    XDG_CONFIG_HOME="$work/config" \
    XDG_RUNTIME_DIR="$work/runtime" \
    XDG_CACHE_HOME="$work/cache" \
    XDG_DATA_HOME="$work/home/.local/share" \
    GSETTINGS_BACKEND=memory \
    IBUS_COMPONENT_PATH="$work/component" \
    RUST_LOG=info \
    E2E_WORK="$work" \
    E2E_BIN_DIR="$bin_dir" \
    dbus-run-session -- "$work/session.sh" >"$work/results.txt" 2>"$work/client.log" || status=$?

cat "$work/results.txt"
if [ "$status" -ne 0 ] || ! grep -q '^pass ' "$work/results.txt" || grep -q '^fail ' "$work/results.txt"; then
    printf '%s\n' "IBus end-to-end test FAILED; logs:" >&2
    for log in client.log ibus-daemon.log; do
        printf '%s\n' "--- $log" >&2
        tail -n 40 "$work/$log" >&2 2>/dev/null || true
    done
    exit 1
fi
printf '%s\n' "IBus end-to-end test passed"
