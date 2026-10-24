#!/usr/bin/env bash
# Headless Sway end-to-end tests.
#
#   scripts/test-sway-e2e.sh [--keep] [SCENARIO...]
#
# Starts a private headless Sway (wlroots headless backend, software
# rendering, no input devices) with its own runtime directory, home and
# configuration, then runs the scenarios in scripts/sway-e2e-session.sh
# against WayExpand's production daemon and wlroots virtual-keyboard
# output. No D-Bus session is started, and nothing touches the running
# desktop, its daemon, or the user's settings.
#
# What this does and does not show: output is typed into real Wayland
# windows by the real wlroots backend, but keyboard capture is simulated
# (the daemon reads stdin), WayExpand has no Sway window tracker (focus is
# untracked), and the session is headless. Results are evidence for the
# output path on this Sway version, not compositor certification.
#
# --keep leaves the work directory (logs, sink states) for inspection.
set -euo pipefail

keep=false
if [ "${1:-}" = --keep ]; then
    keep=true
    shift
fi
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

for tool in sway swaymsg jq mkfifo timeout; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 2
    }
done

cd "$project_dir"
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac
export CARGO_TARGET_DIR="$target_dir"
cargo build --quiet --locked -p wayexpand -p wayexpand-daemon -p wayexpand-e2e \
    --manifest-path "$project_dir/Cargo.toml"
bin="$target_dir/debug"
work=$(mktemp -d "$target_dir/sway-e2e.XXXXXX")
chmod 700 "$work"
# Wayland socket paths must fit in 108 bytes.
runtime=$(mktemp -d /tmp/wxe2e.XXXXXX)
chmod 700 "$runtime"
mkdir -m 700 "$work/home"
results="$work/results.txt"
: >"$results"

sway_pid=
# Stop every process of this user whose HOME is this run's private home --
# no other process can have it.
reap_session_processes() {
    local pid home
    for pid in $(ps -u "$(id -u)" -o pid=); do
        # Other users' and exiting processes cannot be read; skip them.
        # (`|| continue` matters: under `set -e` a failed read would abort
        # this cleanup, leaving the processes and the work directory.)
        home=$({ tr '\0' '\n' <"/proc/$pid/environ"; } 2>/dev/null | sed -n 's/^HOME=//p') ||
            continue
        if [ "$home" = "$work/home" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
}
cleanup() {
    if [ -n "$sway_pid" ]; then
        kill "$sway_pid" 2>/dev/null || true
        wait "$sway_pid" 2>/dev/null || true
    fi
    reap_session_processes
    sleep 0.2
    rm -rf "$runtime"
    if [ "$keep" = true ]; then
        printf 'Kept %s\n' "$work"
    else
        rm -rf "$work"
    fi
}
trap cleanup EXIT

cat >"$work/sway.conf" <<'EOF'
xwayland disable
swaybg_command -
output HEADLESS-1 resolution 1280x800
EOF

session_env=(
    PATH="$PATH"
    HOME="$work/home"
    LANG=C.UTF-8
    XDG_RUNTIME_DIR="$runtime"
    XDG_CONFIG_HOME="$work/home/.config"
    XDG_CACHE_HOME="$work/home/.cache"
    XDG_CURRENT_DESKTOP=sway
)
# A sway unpacked outside the system library path (see
# docs/INTEGRATION_TESTING.md) needs its libraries.
if [ -n "${LD_LIBRARY_PATH:-}" ]; then
    session_env+=(LD_LIBRARY_PATH="$LD_LIBRARY_PATH")
fi

env -i "${session_env[@]}" \
    WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$work/sway.conf" >"$work/sway.log" 2>&1 &
sway_pid=$!

wayland_socket=
for _ in $(seq 50); do
    for candidate in "$runtime"/wayland-*; do
        case "$candidate" in
            *.lock | *'*') ;;
            *) [ -S "$candidate" ] && wayland_socket=${candidate##*/} ;;
        esac
    done
    [ -n "$wayland_socket" ] && break
    sleep 0.1
done
ipc_socket=$(find "$runtime" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit)
if [ -z "$wayland_socket" ] || [ -z "$ipc_socket" ]; then
    printf '%s\n' 'error: the private Sway did not start; compositor log:' >&2
    tail -20 "$work/sway.log" >&2
    exit 2
fi

session_status=0
env -i "${session_env[@]}" \
    WAYLAND_DISPLAY="$wayland_socket" \
    SWAYSOCK="$ipc_socket" \
    E2E_BIN="$bin" \
    E2E_WORK="$work" \
    E2E_RESULTS="$results" \
    E2E_SCENARIOS="$*" \
    timeout 900 "$project_dir/scripts/sway-e2e-session.sh" \
    >"$work/session.out" 2>&1 || session_status=$?

if [ "$session_status" -eq 124 ] || [ "$session_status" -eq 137 ]; then
    printf '%s\n' 'error: the Sway session exceeded its 900s timeout; compositor log:' >&2
    tail -20 "$work/sway.log" >&2
    exit 2
fi
if [ ! -s "$results" ]; then
    printf '%s\n' 'error: the session produced no results; compositor log:' >&2
    tail -20 "$work/sway.log" >&2
    exit 2
fi
pass=0 fail=0 unverified=0
while read -r outcome name detail; do
    printf '%-10s %-38s %s\n' "$outcome" "$name" "$detail"
    case "$outcome" in
        pass) pass=$((pass + 1)) ;;
        fail) fail=$((fail + 1)) ;;
        *) unverified=$((unverified + 1)) ;;
    esac
done <"$results"
if [ -s "$work/runtime-status" ]; then
    status_field() {
        sed -n "s/^$1=//p" "$work/runtime-status"
    }
    printf 'route capture-source %s\n' "$(status_field source)"
    printf 'route injection-backend %s\n' "$(status_field backend)"
    printf 'route insertion-mode %s\n' "$(status_field inject_insertion_mode)"
    printf 'route focus-mode %s\n' "$(status_field compatibility_mode)"
fi
printf '\n%s passed, %s failed, %s unverified (Sway %s, headless, capture simulated, focus untracked)\n' \
    "$pass" "$fail" "$unverified" "$(sway --version 2>/dev/null | awk '{print $3}')"
[ "$fail" -eq 0 ] && [ "$unverified" -eq 0 ]
