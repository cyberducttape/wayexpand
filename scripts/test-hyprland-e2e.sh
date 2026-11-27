#!/usr/bin/env bash
# Nested Hyprland end-to-end tests.
#
#   scripts/test-hyprland-e2e.sh [--keep] [SCENARIO...]
#   E2E_INPUT_SOURCE=input-method scripts/test-hyprland-e2e.sh [--keep]
#
# Hyprland cannot run headless without a DRM device, so this harness runs a
# private Hyprland *nested* in the current Wayland session (it opens one
# window there) with its own runtime directory, home and configuration,
# then runs the scenarios in scripts/hyprland-e2e-session.sh against
# WayExpand's production daemon. Nothing touches the running desktop's
# settings, its WayExpand daemon, or the user's configuration. CI runners
# have neither a GPU nor a parent compositor, so this runs on developer
# machines only.
#
# HYPRLAND may name a Hyprland binary (for example one unpacked from a
# distribution package without installing it); its directory must also
# contain hyprctl. Set LD_LIBRARY_PATH for unpacked libraries.
#
# What this does and does not show: output is typed into real Wayland
# windows, focus comes from the production Hyprland IPC window tracker, and
# with E2E_INPUT_SOURCE=input-method capture uses the production
# input-method-v2 source with its virtual-keyboard pass-through. Keyboard
# input comes from a separate test virtual keyboard, not a physical one.
#
# --keep leaves the work directory (logs, sink states) for inspection.
set -euo pipefail

keep=false
if [ "${1:-}" = --keep ]; then
    keep=true
    shift
fi
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

hyprland=${HYPRLAND:-$(command -v Hyprland || true)}
if [ -z "$hyprland" ] || [ ! -x "$hyprland" ]; then
    printf '%s\n' 'error: Hyprland is required (or set HYPRLAND=/path/to/Hyprland)' >&2
    exit 2
fi
hypr_bin_dir=$(dirname -- "$hyprland")
for tool in jq mkfifo timeout; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 2
    }
done
[ -x "$hypr_bin_dir/hyprctl" ] || command -v hyprctl >/dev/null || {
    printf '%s\n' 'error: hyprctl is required next to Hyprland or on PATH' >&2
    exit 2
}
parent_display=${WAYLAND_DISPLAY:-}
if [ -z "$parent_display" ] || [ -z "${XDG_RUNTIME_DIR:-}" ]; then
    printf '%s\n' 'error: run this from a Wayland session; Hyprland runs nested in it' >&2
    exit 2
fi
case "$parent_display" in
    /*) parent_socket=$parent_display ;;
    *) parent_socket="$XDG_RUNTIME_DIR/$parent_display" ;;
esac
[ -S "$parent_socket" ] || {
    printf 'error: parent Wayland socket %s not found\n' "$parent_socket" >&2
    exit 2
}

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
work=$(mktemp -d "$target_dir/hyprland-e2e.XXXXXX")
chmod 700 "$work"
# Wayland socket paths must fit in 108 bytes.
runtime=$(mktemp -d /tmp/wxe2e.XXXXXX)
chmod 700 "$runtime"
mkdir -m 700 "$work/home"
results="$work/results.txt"
: >"$results"

hyprland_pid=
# Stop every process of this user whose HOME is this run's private home --
# no other process can have it.
reap_session_processes() {
    local pid home
    for pid in $(ps -u "$(id -u)" -o pid=); do
        home=$({ tr '\0' '\n' <"/proc/$pid/environ"; } 2>/dev/null | sed -n 's/^HOME=//p') ||
            continue
        if [ "$home" = "$work/home" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
}
cleanup() {
    if [ -n "$hyprland_pid" ]; then
        kill "$hyprland_pid" 2>/dev/null || true
        wait "$hyprland_pid" 2>/dev/null || true
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

mkdir -p "$work/home/.config/hypr"
cat >"$work/home/.config/hypr/hyprland.conf" <<'EOF'
monitor = , 1280x800, auto, 1
animations {
    enabled = false
}
misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
    disable_autoreload = true
}
ecosystem {
    no_update_news = true
    no_donation_nag = true
}
EOF

session_env=(
    PATH="$hypr_bin_dir:$PATH"
    HOME="$work/home"
    LANG=C.UTF-8
    XDG_RUNTIME_DIR="$runtime"
    XDG_CONFIG_HOME="$work/home/.config"
    XDG_CACHE_HOME="$work/home/.cache"
    XDG_DATA_HOME="$work/home/.local/share"
    XDG_CURRENT_DESKTOP=Hyprland
    # This unmanaged test session has no delegated cgroup; select the
    # compatibility path explicitly.
    WAYEXPAND_PROCESS_CGROUP=disabled
)
if [ -n "${LD_LIBRARY_PATH:-}" ]; then
    session_env+=(LD_LIBRARY_PATH="$LD_LIBRARY_PATH")
fi

# The nested compositor connects to the parent through an absolute socket
# path, so its own clients get a private runtime directory.
env -i "${session_env[@]}" WAYLAND_DISPLAY="$parent_socket" \
    "$hyprland" --config "$work/home/.config/hypr/hyprland.conf" >"$work/hyprland.log" 2>&1 &
hyprland_pid=$!

signature=''
wayland_socket=''
for _ in $(seq 100); do
    for candidate in "$runtime"/hypr/*/.socket2.sock; do
        [ -S "$candidate" ] && signature=$(basename -- "$(dirname -- "$candidate")")
    done
    for candidate in "$runtime"/wayland-*; do
        case "$candidate" in
            *.lock | *'*') ;;
            *) [ -S "$candidate" ] && wayland_socket=${candidate##*/} ;;
        esac
    done
    [ -n "$signature" ] && [ -n "$wayland_socket" ] && break
    sleep 0.1
done
if [ -z "$signature" ] || [ -z "$wayland_socket" ]; then
    printf '%s\n' 'error: the private Hyprland did not start; compositor log:' >&2
    tail -20 "$work/hyprland.log" >&2
    exit 2
fi

session_timeout=$((900 + ${E2E_SOAK_SECONDS:-0}))
session_status=0
env -i "${session_env[@]}" \
    WAYLAND_DISPLAY="$wayland_socket" \
    HYPRLAND_INSTANCE_SIGNATURE="$signature" \
    E2E_BIN="$bin" \
    E2E_WORK="$work" \
    E2E_CONFIG="$runtime/expansions.toml" \
    E2E_RESULTS="$results" \
    E2E_SCENARIOS="$*" \
    E2E_INPUT_SOURCE="${E2E_INPUT_SOURCE:-stdin}" \
    E2E_SOAK_SECONDS="${E2E_SOAK_SECONDS:-90}" \
    E2E_SOAK_RESTART_EVERY="${E2E_SOAK_RESTART_EVERY:-1}" \
    E2E_SOAK_MAX_RSS_GROWTH_KIB="${E2E_SOAK_MAX_RSS_GROWTH_KIB:-65536}" \
    timeout "$session_timeout" "$project_dir/scripts/hyprland-e2e-session.sh" \
    >"$work/session.out" 2>&1 || session_status=$?

if [ "$session_status" -eq 124 ] || [ "$session_status" -eq 137 ]; then
    printf '%s\n' 'error: the Hyprland session exceeded its timeout; compositor log:' >&2
    tail -20 "$work/hyprland.log" >&2
    exit 2
fi
if [ ! -s "$results" ]; then
    printf '%s\n' 'error: the session produced no results; compositor log:' >&2
    tail -20 "$work/hyprland.log" >&2
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
case "${E2E_INPUT_SOURCE:-stdin}" in
    input-method) capture_label='input-method-v2 capture' ;;
    *) capture_label='capture simulated' ;;
esac
printf '\n%s passed, %s failed, %s unverified (Hyprland %s, nested, %s, focus tracked over IPC)\n' \
    "$pass" "$fail" "$unverified" \
    "$("$hyprland" --version 2>/dev/null | awk 'NR == 1 {print $2}')" "$capture_label"
[ "$fail" -eq 0 ] && [ "$unverified" -eq 0 ]
