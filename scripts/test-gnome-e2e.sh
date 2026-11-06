#!/bin/sh
# End-to-end test of GNOME window tracking: a private, headless GNOME Shell
# runs the bundled WayExpand extension, Wayland windows take focus, and
# WayExpand's GNOME tracker must report their app IDs. Nothing touches the
# desktop session: the shell gets its own D-Bus session, HOME, runtime
# directory, data directory (with the extension) and dconf settings.
#
# Usage: scripts/test-gnome-e2e.sh [--keep]
#   E2E_BIN=DIR        use prebuilt wx-e2e-sink and wx-e2e-gnome-tracker
#   E2E_SKIP_BUILD=1   do not run cargo build
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck disable=SC1091
. "$project_dir/scripts/gnome-extension-state.sh"
uuid=wayexpand-window-tracker@cyberducttape.github.io
controller_uuid=wayexpand-e2e-focus-controller@cyberducttape.github.io
keep=0
for argument in "$@"; do
    case "$argument" in
        --keep) keep=1 ;;
        *) printf '%s\n' "usage: $0 [--keep]" >&2; exit 2 ;;
    esac
done
for tool in gnome-shell dbus-run-session gsettings gdbus; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf '%s\n' "error: $tool is required (install gnome-shell, dbus-daemon and libglib2.0-bin)" >&2
        exit 2
    fi
done

bin_dir=${E2E_BIN:-"$project_dir/target/debug"}
if [ "${E2E_SKIP_BUILD:-0}" != 1 ]; then
    cargo build --locked --quiet -p wayexpand-e2e --bin wx-e2e-sink --bin wx-e2e-gnome-tracker
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-gnome-e2e.XXXXXX")
# Wayland socket paths must stay short, so the runtime directory is separate.
runtime=$(mktemp -d /tmp/wxgnome.XXXXXX)
cleanup() {
    if [ "$keep" -eq 1 ]; then
        printf '%s\n' "kept $work"
    else
        rm -rf -- "$work"
    fi
    rm -rf -- "$runtime"
}
trap cleanup EXIT INT TERM
chmod 0700 "$runtime"
mkdir -p "$work/home" "$work/config" "$work/cache" \
    "$work/data/gnome-shell/extensions/$uuid" \
    "$work/data/gnome-shell/extensions/$controller_uuid"
cp "$project_dir/gnome-extension/$uuid/metadata.json" "$project_dir/gnome-extension/$uuid/extension.js" \
    "$work/data/gnome-shell/extensions/$uuid/"
cp "$project_dir/scripts/gnome-e2e-focus-controller/metadata.json" \
    "$project_dir/scripts/gnome-e2e-focus-controller/extension.js" \
    "$work/data/gnome-shell/extensions/$controller_uuid/"

cat >"$work/session.sh" <<'EOF'
#!/bin/sh
set -eu
. "$E2E_STATE_HELPER"
. "$E2E_DBUS_HELPER"
display=wayexpand-gnome-e2e
log() { printf '%s\n' "$*"; }
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$E2E_UUID', '$E2E_CONTROLLER_UUID']"
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell.log" 2>&1 &
shell_pid=$!
sinks=
finish() {
    for pid in $sinks; do kill "$pid" 2>/dev/null || true; done
    kill "$shell_pid" 2>/dev/null || true
    wait "$shell_pid" 2>/dev/null || true
}
trap finish EXIT

extension_state() {
    gnome_shell_extensions_call GetExtensionInfo "$E2E_UUID" || true
}
tries=600
until gnome_extension_state_active "$(extension_state)"; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        if kill -0 "$shell_pid" 2>/dev/null; then
            shell_state=running
        else
            shell_state=exited
        fi
        log "fail extension-active GNOME Shell did not activate the extension (shell=$shell_state): $(extension_state)"
        exit 1
    fi
    sleep 0.1
done
log "pass extension-active"

start_sink() {
    WAYLAND_DISPLAY="$display" "$E2E_BIN_DIR/wx-e2e-sink" "$1" "$E2E_WORK/$1.json" \
        >>"$E2E_WORK/sink.log" 2>&1 &
    sinks="$sinks $!"
    last_sink=$!
}

start_transition_tracker() {
    mode=$1
    target=$2
    label=$3
    tracker_log="$E2E_WORK/tracker-$label.log"
    if [ -n "$target" ]; then
        "$E2E_BIN_DIR/wx-e2e-gnome-tracker" "$mode" "$target" \
            >"$tracker_log" 2>&1 &
    else
        "$E2E_BIN_DIR/wx-e2e-gnome-tracker" "$mode" >"$tracker_log" 2>&1 &
    fi
    tracker_pid=$!
    tries=500
    until grep -q '^ready ' "$tracker_log"; do
        tries=$((tries - 1))
        if [ "$tries" -le 0 ] || ! kill -0 "$tracker_pid" 2>/dev/null; then
            cat "$tracker_log"
            log "fail tracker-ready GNOME tracker did not subscribe before the focus action"
            return 1
        fi
        sleep 0.05
    done
}

activate_window() {
    tries=100
    while :; do
        if gdbus call --session --dest org.gnome.Shell \
            --object-path /org/wayexpand/GnomeE2eFocusController \
            --method org.wayexpand.GnomeE2eFocusController.ActivateWindow "$1" \
            >/dev/null 2>&1; then
            return 0
        fi
        tries=$((tries - 1))
        [ "$tries" -gt 0 ] || return 1
        sleep 0.05
    done
}

check_focus_transition() {
    app_id=$1
    label=$2
    if ! start_transition_tracker --expect-transition "$app_id" "$label"; then
        return 1
    fi
    if ! activate_window "$app_id"; then
        kill "$tracker_pid" 2>/dev/null || true
        wait "$tracker_pid" 2>/dev/null || true
        cat "$E2E_WORK/tracker-$label.log"
        return 1
    fi
    tracker_status=0
    wait "$tracker_pid" || tracker_status=$?
    cat "$E2E_WORK/tracker-$label.log"
    return "$tracker_status"
}

status=0
start_sink org.wayexpand.GnomeE2eFirst
start_sink org.wayexpand.GnomeE2eSecond
# Establish a known baseline before checking transitions. This avoids relying
# on whichever client Mutter happens to focus when the headless session starts.
activate_window org.wayexpand.GnomeE2eSecond || status=1
check_focus_transition org.wayexpand.GnomeE2eFirst first || status=1
check_focus_transition org.wayexpand.GnomeE2eSecond second || status=1

# A headless client cannot steal focus from another toplevel without a user
# activation token, so a test-only GNOME Shell extension performs activation.
# This exercises actual focus notifications without claiming that a real user
# session or physical activation flow is certified.
if start_transition_tracker --expect-transition-empty "" empty; then
    for pid in $sinks; do kill "$pid" 2>/dev/null || true; done
    for pid in $sinks; do wait "$pid" 2>/dev/null || true; done
    tracker_status=0
    wait "$tracker_pid" || tracker_status=$?
    cat "$E2E_WORK/tracker-empty.log"
    [ "$tracker_status" -eq 0 ] || status=1
else
    status=1
fi

if ! gnome_shell_extensions_call DisableExtension "$E2E_UUID" >/dev/null; then
    log "fail extension-disabled could not disable the extension through GNOME Shell D-Bus"
    exit 1
fi
tries=100
until gnome_extension_state_inactive "$(extension_state)"; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        log "fail extension-disabled GNOME Shell did not disable the extension: $(extension_state)"
        status=1
        break
    fi
    sleep 0.1
done
if [ "$tries" -gt 0 ]; then
    log "pass extension-disabled"
fi
"$E2E_BIN_DIR/wx-e2e-gnome-tracker" --expect-unavailable || status=1
exit "$status"
EOF
chmod 0755 "$work/session.sh"

status=0
env -i \
    HOME="$work/home" \
    PATH="$PATH" \
    XDG_CONFIG_HOME="$work/config" \
    XDG_CACHE_HOME="$work/cache" \
    XDG_DATA_HOME="$work/data" \
    XDG_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" \
    XDG_RUNTIME_DIR="$runtime" \
    XDG_CURRENT_DESKTOP=GNOME \
    XDG_SESSION_TYPE=wayland \
    E2E_WORK="$work" \
    E2E_BIN_DIR="$bin_dir" \
    E2E_UUID="$uuid" \
    E2E_CONTROLLER_UUID="$controller_uuid" \
    E2E_STATE_HELPER="$project_dir/scripts/gnome-extension-state.sh" \
    E2E_DBUS_HELPER="$project_dir/scripts/gnome-extension-dbus.sh" \
    dbus-run-session -- "$work/session.sh" >"$work/results.txt" 2>"$work/session.log" || status=$?

cat "$work/results.txt"
if [ "$status" -ne 0 ] || grep -q '^fail' "$work/results.txt" \
    || [ "$(grep -c '^pass' "$work/results.txt")" -ne 6 ]; then
    printf '%s\n' "GNOME window-tracking end-to-end test FAILED; logs:" >&2
    failure=$(grep '^fail ' "$work/results.txt" | tail -n 1 || true)
    if [ -z "$failure" ]; then
        failure="test exited with status $status; pass count $(grep -c '^pass' "$work/results.txt")"
    fi
    failure=$(printf '%s' "$failure" | sed 's/%/%25/g; s/\r/%0D/g; s/\n/%0A/g')
    printf '::error title=GNOME window tracking E2E::%s\n' "$failure" >&2
    for log in session.log gnome-shell.log sink.log; do
        printf '%s\n' "--- $log" >&2
        tail -n 60 "$work/$log" >&2 2>/dev/null || true
        if [ -s "$work/$log" ]; then
            log_summary=$(tail -n 12 "$work/$log" | tr '\n' ';' | sed 's/%/%25/g; s/\r/%0D/g')
            printf '::error title=GNOME E2E %s::%s\n' "$log" "$log_summary" >&2
        fi
    done
    exit 1
fi
printf '%s\n' "GNOME window-tracking end-to-end test passed"
