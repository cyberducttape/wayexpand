#!/usr/bin/env bash
# Headless KWin end-to-end tests.
#
#   scripts/test-kwin-e2e.sh [--keep] [--allow-unverified-unicode] [SCENARIO...]
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
allow_unicode_unverified=false
input_source=stdin
while [ "$#" -gt 0 ]; do
    case "$1" in
        --keep) keep=true; shift ;;
        --allow-unverified-unicode) allow_unicode_unverified=true; shift ;;
        --source=stdin|--source=input-method)
            input_source=${1#--source=}
            shift
            ;;
        --source)
            input_source=${2:?missing value for --source}
            [ "$input_source" = stdin ] || [ "$input_source" = input-method ] || {
                printf '%s\n' 'error: --source must be stdin or input-method' >&2
                exit 2
            }
            shift 2
            ;;
        *) break ;;
    esac
done
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
    # Installed layout: PREFIX/libexec/wayexpand-certification/bin -> PREFIX/bin.
    app_bin=${WAYEXPAND_E2E_APP_BIN:-"$bin/../../../bin"}
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
    source_tree_state=unknown
else
    # Pick up the repository's rust-toolchain.toml even when the user invoked
    # doctor from another working directory.
    cd "$project_dir"
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        # Only files that can affect the binaries or this harness determine
        # whether the tested source is clean. Unrelated files in the user's
        # repository root (for example a generated report) must not make an
        # otherwise matching CLI/daemon build unverifiable.
        source_tree_state=$(sh "$project_dir/scripts/source-tree-state.sh" "$project_dir" \
            Cargo.toml Cargo.lock .cargo rust-toolchain.toml crates scripts tests/certification)
    else
        source_tree_state=unknown
    fi
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

# Bind the evidence to the actual CLI/daemon binaries under test. In a source
# checkout this script rebuilds the binaries, so the caller's CLI may be stale.
cli_version=$("$app_bin/wayexpand" --version) || {
    printf '%s\n' 'error: could not read the tested WayExpand CLI build identity' >&2
    exit 2
}
daemon_version=$("$app_bin/wayexpand-daemon" --version) || {
    printf '%s\n' 'error: could not read the tested WayExpand daemon build identity' >&2
    exit 2
}
cli_commit=$(printf '%s\n' "$cli_version" |
    sed -n 's/.* (commit \([^)]*\))$/\1/p')
daemon_commit=$(printf '%s\n' "$daemon_version" |
    sed -n 's/.* (commit \([^)]*\))$/\1/p')
if [ -z "$cli_commit" ] || [ "$cli_commit" = unknown ] ||
    [ -z "$daemon_commit" ] || [ "$daemon_commit" = unknown ] ||
    [ "$cli_commit" != "$daemon_commit" ]; then
    printf '%s\n' 'error: tested CLI and daemon build identities do not match' >&2
    exit 2
fi
printf 'tested-build commit %s\n' "$cli_commit"
printf 'tested-build source-tree %s\n' "$source_tree_state"

session_status=0
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
    E2E_INPUT_SOURCE="$input_source" \
    E2E_SCENARIOS="$*" \
    timeout 900 dbus-run-session -- \
    kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --width 1280 --height 800 --socket wayexpand-e2e \
    --exit-with-session "$session_driver" \
    >"$work/kwin.log" 2>&1 || session_status=$?

if [ "$session_status" -eq 124 ] || [ "$session_status" -eq 137 ]; then
    printf '%s\n' 'error: private KWin session exceeded its 900s timeout; compositor log:' >&2
    tail -20 "$work/kwin.log" >&2
    exit 2
fi

if [ ! -s "$results" ]; then
    printf '%s\n' 'error: the session produced no results; compositor log:' >&2
    tail -20 "$work/kwin.log" >&2
    exit 2
fi
pass=0 fail=0 unverified=0 required_unverified=0
while read -r outcome name detail; do
    printf '%-10s %-38s %s\n' "$outcome" "$name" "$detail"
    case "$outcome" in
        pass) pass=$((pass + 1)) ;;
        fail) fail=$((fail + 1)) ;;
        *)
            unverified=$((unverified + 1))
            [ "$name" = unicode-insertion ] || required_unverified=$((required_unverified + 1))
            ;;
    esac
done <"$results"
if [ -s "$work/runtime-status" ]; then
    status_field() {
        sed -n "s/^$1=//p" "$work/runtime-status"
    }
    printf 'route capture-source %s\n' "$(status_field source)"
    printf 'route injection-backend %s\n' "$(status_field backend)"
    printf 'route insertion-mode %s\n' "$(status_field inject_insertion_mode)"
    printf 'route backend-mode %s\n' "$(status_field backend_mode)"
    printf 'route sensitive-field-detection %s\n' "$(status_field capture_sensitive_focus)"
fi
printf '\n%s passed, %s failed, %s unverified (KWin %s, headless, capture simulated)\n' \
    "$pass" "$fail" "$unverified" "$(kwin_wayland --version 2>/dev/null | awk '{print $2}')"
[ "$fail" -eq 0 ] && {
    [ "$unverified" -eq 0 ] || {
        [ "$allow_unicode_unverified" = true ] && [ "$required_unverified" -eq 0 ]
    }
}
