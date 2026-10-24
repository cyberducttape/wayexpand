#!/usr/bin/env bash
# Headless KWin end-to-end tests.
#
#   scripts/test-kwin-e2e.sh [--keep] [SCENARIO...]
#
# Starts a private KWin (kwin_wayland --virtual) with its own D-Bus session,
# runtime directory, home and configuration, then runs the scenarios in
# scripts/kwin-e2e-session.sh against WayExpand's production daemon, libei
# output and KWin window tracker. Nothing touches the running desktop, its
# daemon, or the user's settings.
#
# What this does and does not show: output is typed into real Wayland
# windows by the real libei backend, and focus comes from the real KWin
# tracker, but keyboard capture is simulated (the daemon reads stdin) and
# the session is headless. Results are evidence for the output and focus
# paths on this KWin version, not compositor certification.
#
# --keep leaves the work directory (logs, sink states) for inspection.
set -euo pipefail

keep=false
if [ "${1:-}" = --keep ]; then
    keep=true
    shift
fi
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
session_driver=$(dirname -- "$0")/kwin-e2e-session.sh
if [ ! -f "$session_driver" ]; then
    session_driver="$project_dir/scripts/kwin-e2e-session.sh"
fi

for tool in kwin_wayland dbus-run-session gdbus jq mkfifo timeout; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 2
    }
done

if [ -n "${WAYEXPAND_E2E_PREBUILT_BIN:-}" ]; then
    bin=$WAYEXPAND_E2E_PREBUILT_BIN
    app_bin=${WAYEXPAND_E2E_APP_BIN:-"$bin/../../../../bin"}
    for executable in wx-e2e-sink wx-e2e-type wx-e2e-eis-relay; do
        [ -x "$bin/$executable" ] || {
            printf 'error: prebuilt certification helper is missing: %s\n' "$executable" >&2
            exit 2
        }
    done
    for executable in wayexpand wayexpand-daemon; do
        [ -x "$app_bin/$executable" ] || {
            printf 'error: installed WayExpand binary is missing: %s\n' "$executable" >&2
            exit 2
        }
    done
    work_parent=${XDG_CACHE_HOME:-"$HOME/.cache"}/wayexpand/certification
    mkdir -p "$work_parent"
    chmod 700 "$work_parent"
    work=$(mktemp -d "$work_parent/kwin-e2e.XXXXXX")
else
    # Pick up the repository's rust-toolchain.toml even when the user invoked
    # doctor from another working directory.
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
    app_bin=$bin
    # Restrict this test's files, not a possibly shared CARGO_TARGET_DIR.
    work=$(mktemp -d "$target_dir/kwin-e2e.XXXXXX")
fi
chmod 700 "$work"
# Wayland and libei socket paths must fit in 108 bytes.
runtime=$(mktemp -d /tmp/wxe2e.XXXXXX)
# KWin's own start-up D-Bus-activates the desktop portal, which brings up
# xdg-desktop-portal-kde and ksecretd; they outlive the private bus. Stop
# every process of this user whose HOME is this run's private home -- no
# other process can have it.
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
chmod 700 "$work" "$runtime"
mkdir -m 700 "$work/home"
results="$work/results.txt"
: >"$results"

env -i \
    PATH="$PATH" \
    HOME="$work/home" \
    LANG=C.UTF-8 \
    XDG_RUNTIME_DIR="$runtime" \
    XDG_CONFIG_HOME="$work/home/.config" \
    XDG_CACHE_HOME="$work/home/.cache" \
    XDG_CURRENT_DESKTOP=KDE \
    KDE_SESSION_VERSION=6 \
    QT_QPA_PLATFORM=offscreen \
    E2E_BIN="$bin" \
    E2E_APP_BIN="$app_bin" \
    E2E_WORK="$work" \
    E2E_RESULTS="$results" \
    E2E_SCENARIOS="$*" \
    timeout 900 dbus-run-session -- \
    kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --width 1280 --height 800 --socket wayexpand-e2e \
    --exit-with-session "$session_driver" \
    >"$work/kwin.log" 2>&1 || true

if [ ! -s "$results" ]; then
    printf '%s\n' 'error: the session produced no results; compositor log:' >&2
    tail -20 "$work/kwin.log" >&2
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
printf '\n%s passed, %s failed, %s unverified (KWin %s, headless, capture simulated)\n' \
    "$pass" "$fail" "$unverified" "$(kwin_wayland --version 2>/dev/null | awk '{print $2}')"
[ "$fail" -eq 0 ] && [ "$unverified" -eq 0 ]
