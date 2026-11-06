#!/bin/sh
# End-to-end test of GNOME source activation, the IBus engine, and window
# tracking in a private headless GNOME Shell session. It verifies GNOME source
# registration/activation followed by an IBus replacement transaction and
# focus tracking. The replacement uses a controlled IBus protocol client,
# not a GTK application, so this is not desktop certification.
#
# Usage: scripts/test-gnome-e2e.sh [--keep]
#   E2E_BIN=DIR        use prebuilt wayexpand-ibus and wx-e2e-* binaries
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
    cargo build --locked --quiet \
        -p wayexpand-backend-ibus --bin wayexpand-ibus \
        -p wayexpand-e2e --bin wx-e2e-sink --bin wx-e2e-gnome-tracker --bin wx-e2e-ibus
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
mkdir -p "$work/home" "$work/config/wayexpand" "$work/cache" "$work/component" \
    "$work/data/gnome-shell/extensions/$uuid" \
    "$work/data/gnome-shell/extensions/$controller_uuid"
cp "$project_dir/gnome-extension/$uuid/metadata.json" "$project_dir/gnome-extension/$uuid/extension.js" \
    "$project_dir/gnome-extension/$uuid/notification-queue.js" \
    "$work/data/gnome-shell/extensions/$uuid/"
cp "$project_dir/scripts/gnome-e2e-focus-controller/metadata.json" \
    "$project_dir/scripts/gnome-e2e-focus-controller/extension.js" \
    "$work/data/gnome-shell/extensions/$controller_uuid/"
cat >"$work/config/wayexpand/expansions.toml" <<'EOF'
[settings]
compatibility_mode = "tracked-focus"

[[expansion]]
trigger = ";e2e"
replacement = "expanded"
EOF
chmod 0700 "$work/config" "$work/config/wayexpand"
chmod 0600 "$work/config/wayexpand/expansions.toml"
sed "s|<exec>wayexpand-ibus</exec>|<exec>$bin_dir/wayexpand-ibus</exec>|" \
    "$project_dir/desktop/wayexpand-ibus.xml" >"$work/component/wayexpand.xml"
chmod 0700 "$work/component"

cat >"$work/session.sh" <<'EOF'
#!/bin/sh
set -eu
. "$E2E_STATE_HELPER"
. "$E2E_DBUS_HELPER"
display=wayexpand-gnome-e2e
log() { printf '%s\n' "$*"; }
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$E2E_UUID', '$E2E_CONTROLLER_UUID']"
gsettings set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('xkb', 'de')]"
socket="$XDG_RUNTIME_DIR/ibus.sock"
ibus-daemon --single --panel=disable --emoji-extension=disable --config=disable \
    --cache=none --address="unix:path=$socket" >"$E2E_WORK/ibus-daemon.log" 2>&1 &
ibus_pid=$!
tries=100
while [ ! -S "$socket" ] && [ "$tries" -gt 0 ]; do
    sleep 0.1
    tries=$((tries - 1))
done
if [ ! -S "$socket" ]; then
    log "fail ibus-started private ibus-daemon did not create its socket"
    kill "$ibus_pid" 2>/dev/null || true
    exit 1
fi
export IBUS_ADDRESS="unix:path=$socket"
if ! ibus list-engine --name-only >"$E2E_WORK/ibus-engines.txt" 2>&1 \
    || ! grep -Fx wayexpand "$E2E_WORK/ibus-engines.txt" >/dev/null; then
    log "fail ibus-engine-registered WayExpand engine is missing from the private IBus registry"
    cat "$E2E_WORK/ibus-engines.txt"
    kill "$ibus_pid" 2>/dev/null || true
    exit 1
fi
log "pass ibus-engine-registered"
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell.log" 2>&1 &
shell_pid=$!
sinks=
finish() {
    for pid in $sinks; do kill "$pid" 2>/dev/null || true; done
    kill "$shell_pid" 2>/dev/null || true
    wait "$shell_pid" 2>/dev/null || true
    if [ -f "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json" ]; then
        engine_pid=$(sed -n 's/.*"pid":\([0-9][0-9]*\).*/\1/p' \
            "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json")
        if [ -n "$engine_pid" ] \
            && [ "$(cat "/proc/$engine_pid/comm" 2>/dev/null)" = wayexpand-ibus ]; then
            kill "$engine_pid" 2>/dev/null || true
        fi
    fi
    kill "$ibus_pid" 2>/dev/null || true
    wait "$ibus_pid" 2>/dev/null || true
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

# Verify source activation and readback through GNOME Shell's live
# InputSourceManager. Then write the deprecated `current` key to request a
# different source and assert that Shell's observed active source is unchanged.
gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.ActivateInputSource xkb de \
    >"$E2E_WORK/source-activation.log"
active_source=$(gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.GetActiveInputSource)
case "$active_source" in
    *"'xkb', 'de'"*) log "pass active-input-source $active_source" ;;
    *) log "fail active-input-source GNOME Shell reported $active_source"; exit 1 ;;
esac
if ! gsettings set org.gnome.desktop.input-sources current 0 \
    >"$E2E_WORK/deprecated-current.log" 2>&1; then
    log "fail deprecated-current-setting could not write the legacy key in the isolated session"
    exit 1
fi
configured_current=$(gsettings get org.gnome.desktop.input-sources current)
case "$configured_current" in
    *0*) log "pass deprecated-current-readback $configured_current" ;;
    *) log "fail deprecated-current-readback unexpected value $configured_current"; exit 1 ;;
esac
active_source=$(gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.GetActiveInputSource)
case "$active_source" in
    *"'xkb', 'de'"*) log "pass ignored-current-setting $active_source" ;;
    *) log "fail ignored-current-setting GNOME Shell reported $active_source"; exit 1 ;;
esac
if gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.ActivateInputSource ibus wayexpand \
    >/dev/null 2>&1; then
    log "fail unavailable-input-source GNOME activated an unregistered IBus engine"
    exit 1
fi
log "pass unavailable-input-source"

# Register in GNOME's typed source list without disturbing the keyboard
# layouts, activate through Shell's live manager, then prove the already
# selected IBus engine performs a trigger replacement in its input context.
gsettings set org.gnome.desktop.input-sources sources \
    "[('xkb', 'us'), ('xkb', 'de'), ('ibus', 'wayexpand')]"
registered_sources=$(gsettings get org.gnome.desktop.input-sources sources)
case "$registered_sources" in
    *"'ibus', 'wayexpand'"*) log "pass gnome-ibus-source-registered $registered_sources" ;;
    *) log "fail gnome-ibus-source-registered GNOME returned $registered_sources"; exit 1 ;;
esac
gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.ActivateInputSource ibus wayexpand \
    >"$E2E_WORK/ibus-source-activation.log"
active_source=$(gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.wayexpand.ShellWindowTracker1.GetActiveInputSource)
case "$active_source" in
    *"'ibus', 'wayexpand'"*) log "pass gnome-ibus-source-active $active_source" ;;
    *) log "fail gnome-ibus-source-active GNOME Shell reported $active_source"; exit 1 ;;
esac
if ! "$E2E_BIN_DIR/wx-e2e-ibus" "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json" \
    --expect-active >"$E2E_WORK/ibus-transaction-results.txt" 2>&1; then
    log "fail ibus-expansion-after-gnome-activation"
    cat "$E2E_WORK/ibus-transaction-results.txt"
    exit 1
fi
if ! grep -Fx 'pass gnome-selected-engine the engine selected by GNOME was already active in IBus' \
    "$E2E_WORK/ibus-transaction-results.txt" >/dev/null \
    || ! grep -F 'pass expands text after typing the trigger: "expanded"' \
        "$E2E_WORK/ibus-transaction-results.txt" >/dev/null; then
    log "fail ibus-expansion-after-gnome-activation GNOME-selected engine did not expand the trigger"
    cat "$E2E_WORK/ibus-transaction-results.txt"
    exit 1
fi
log "pass ibus-expansion-after-gnome-activation GTK_UNVERIFIED"

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
    IBUS_COMPONENT_PATH="$work/component" \
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
expected_checks='extension-active active-input-source deprecated-current-readback ignored-current-setting unavailable-input-source ibus-engine-registered gnome-ibus-source-registered gnome-ibus-source-active ibus-expansion-after-gnome-activation extension-disabled'
missing_check=
for check in $expected_checks; do
    if ! grep -F "pass $check" "$work/results.txt" >/dev/null; then
        missing_check=$check
        break
    fi
done
if [ "$status" -ne 0 ] || grep -q '^fail' "$work/results.txt" \
    || [ -n "$missing_check" ] \
    || ! grep -F 'pass focus transition to org.wayexpand.GnomeE2eFirst' "$work/results.txt" >/dev/null \
    || ! grep -F 'pass focus transition to org.wayexpand.GnomeE2eSecond' "$work/results.txt" >/dev/null \
    || ! grep -F 'pass focus-loss transition' "$work/results.txt" >/dev/null; then
    printf '%s\n' "GNOME window-tracking end-to-end test FAILED; logs:" >&2
    failure=$(grep '^fail ' "$work/results.txt" | tail -n 1 || true)
    if [ -z "$failure" ]; then
        failure="test exited with status $status; missing checkpoint=${missing_check:-focus-transition}; pass count $(grep -c '^pass' "$work/results.txt")"
    fi
    failure=$(printf '%s' "$failure" | sed 's/%/%25/g; s/\r/%0D/g; s/\n/%0A/g')
    printf '::error title=GNOME window tracking E2E::%s\n' "$failure" >&2
    for log in session.log gnome-shell.log ibus-daemon.log ibus-engines.txt \
        ibus-transaction-results.txt sink.log; do
        printf '%s\n' "--- $log" >&2
        tail -n 60 "$work/$log" >&2 2>/dev/null || true
        if [ -s "$work/$log" ]; then
            log_summary=$(tail -n 12 "$work/$log" | tr '\n' ';' | sed 's/%/%25/g; s/\r/%0D/g')
            printf '::error title=GNOME E2E %s::%s\n' "$log" "$log_summary" >&2
        fi
    done
    exit 1
fi
printf '%s\n' "Headless GNOME Shell and IBus integration passed; GTK application insertion remains unverified"
