#!/bin/sh
# End-to-end test of GNOME source activation, the IBus engine, and window
# tracking in a private headless GNOME Shell session. It verifies GNOME source
# registration/activation, an IBus protocol transaction, insertion into a GTK
# 3 entry, focus tracking, and extension upgrade/reload behavior. The GTK
# test uses a CI uinput keyboard; it is not physical-keyboard certification.
#
# Usage: scripts/test-gnome-e2e.sh [--keep]
#   E2E_BIN=DIR        use prebuilt wayexpand, wayexpand-ibus, and wx-e2e-* binaries
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
        -p wayexpand --bin wayexpand \
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
    "$work/home/.local/share/gnome-shell/extensions/$uuid" \
    "$work/home/.local/share/gnome-shell/extensions/$controller_uuid"
cp "$project_dir/gnome-extension/$uuid/metadata.json" "$project_dir/gnome-extension/$uuid/extension.js" \
    "$project_dir/gnome-extension/$uuid/notification-queue.js" \
    "$work/home/.local/share/gnome-shell/extensions/$uuid/"
python3 - "$work/home/.local/share/gnome-shell/extensions/$uuid/metadata.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as source:
    metadata = json.load(source)
metadata["x-wayexpand-build-id"] = "sha256:pre-upgrade-e2e-fixture"
with open(path, "w", encoding="utf-8") as destination:
    json.dump(metadata, destination)
    destination.write("\n")
PY
cp "$project_dir/scripts/gnome-e2e-focus-controller/metadata.json" \
    "$project_dir/scripts/gnome-e2e-focus-controller/extension.js" \
    "$work/home/.local/share/gnome-shell/extensions/$controller_uuid/"
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
if [ "$HOME" != "$E2E_WORK/home" ] \
    || [ "$XDG_CONFIG_HOME" != "$E2E_WORK/config" ] \
    || [ "$XDG_DATA_HOME" != "$HOME/.local/share" ]; then
    log "fail session-isolation GNOME test escaped its private HOME/XDG directories"
    exit 1
fi
if ! command -v wayexpand-ibus >"$E2E_WORK/wayexpand-ibus-path.txt"; then
    log "fail ibus-engine-executable WayExpand's IBus engine executable is not on PATH"
    exit 1
fi
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions "['$E2E_UUID', '$E2E_CONTROLLER_UUID']"
gsettings set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('xkb', 'de')]"
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell.log" 2>&1 &
shell_pid=$!
# GNOME Shell owns the session IBus lifecycle. Starting a second private
# daemon before Shell causes Shell to replace it, leaving later CLI probes
# pointed at a stale IBUS_ADDRESS. Let Shell start its daemon, then verify the
# component registered in that live session.
tries=100
while :; do
    if ibus list-engine --name-only >"$E2E_WORK/ibus-engines.txt" 2>&1 \
        && grep -Fx wayexpand "$E2E_WORK/ibus-engines.txt" >/dev/null; then
        log "pass ibus-engine-registered"
        break
    fi
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        log "fail ibus-engine-registered GNOME's session IBus registry did not load WayExpand"
        cat "$E2E_WORK/ibus-engines.txt"
        exit 1
    fi
    sleep 0.1
done
# Preserve safe activation diagnostics: configuration values, extension IDs,
# and paths only. This helps distinguish an undiscovered extension from one
# that GNOME refused to enable; no snippet or keyboard data is collected.
gsettings get org.gnome.shell enabled-extensions >"$E2E_WORK/enabled-extensions.txt" 2>&1 || true
gnome-extensions list >"$E2E_WORK/installed-extensions.txt" 2>&1 || true
find "$HOME/.local/share/gnome-shell/extensions" -mindepth 1 -maxdepth 2 \
    -type f -name metadata.json -print >"$E2E_WORK/user-extension-metadata-paths.txt" 2>&1 || true
# Ask GNOME's extension manager to activate both installed test extensions.
# This is the same supported activation API the user can reach with
# `gnome-extensions enable`; it also avoids racing the asynchronous dconf
# writer immediately after setting enabled-extensions above.
for extension_uuid in "$E2E_UUID" "$E2E_CONTROLLER_UUID"; do
    tries=100
    while ! gnome_shell_extensions_call EnableExtension "$extension_uuid" \
        >"$E2E_WORK/enable-$extension_uuid.log" 2>&1; do
        tries=$((tries - 1))
        if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
            log "fail extension-enable GNOME Shell could not enable $extension_uuid: $(cat "$E2E_WORK/enable-$extension_uuid.log")"
            exit 1
        fi
        sleep 0.1
    done
done
sinks=
gtk_pid=
finish() {
    for pid in $sinks; do kill "$pid" 2>/dev/null || true; done
    if [ -n "$gtk_pid" ]; then
        kill "$gtk_pid" 2>/dev/null || true
        wait "$gtk_pid" 2>/dev/null || true
    fi
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
}
trap finish EXIT

extension_state() {
    gnome_shell_extensions_call GetExtensionInfo "$E2E_UUID" || true
}

activate_window() {
    tries=100
    activate_error="$E2E_WORK/activate-window.err"
    while :; do
        if gdbus call --session --dest org.gnome.Shell \
            --object-path /org/wayexpand/GnomeE2eFocusController \
            --method org.wayexpand.GnomeE2eFocusController.ActivateWindow "$1" \
            >/dev/null 2>"$activate_error"; then
            return 0
        fi
        # The controller already spent up to five seconds asking Mutter to
        # focus a mapped window. Retrying a focus timeout multiplies that
        # bound by 100 and hides the actual failure; only retry transient
        # lookup/session startup errors.
        if grep -Eq 'FocusTimeout|did not focus' "$activate_error"; then
            cat "$activate_error" >&2
            return 1
        fi
        tries=$((tries - 1))
        if [ "$tries" -le 0 ]; then
            cat "$activate_error" >&2
            return 1
        fi
        sleep 0.05
    done
}

run_gtk_insertion() {
    label=$1
    gtk_log="$E2E_WORK/gtk-insertion-$label.log"
    WAYLAND_DISPLAY="$display" /usr/bin/python3 \
        "$E2E_PROJECT_DIR/scripts/certification-clients/gtk_ibus_insertion.py" \
        >"$gtk_log" 2>&1 &
    gtk_pid=$!
    tries=200
    until grep -Fx 'ready gtk-ibus-insertion org.wayexpand.GtkIbusInsertion' "$gtk_log" >/dev/null 2>&1; do
        tries=$((tries - 1))
        if [ "$tries" -le 0 ] || ! kill -0 "$gtk_pid" 2>/dev/null; then
            cat "$gtk_log" 2>/dev/null || true
            log "fail gtk-ibus-insertion-$label GTK test application did not become ready"
            kill "$gtk_pid" 2>/dev/null || true
            wait "$gtk_pid" 2>/dev/null || true
            gtk_pid=
            return 1
        fi
        sleep 0.05
    done
    if ! activate_window org.wayexpand.GtkIbusInsertion; then
        log "fail gtk-ibus-insertion-$label GNOME Shell could not focus the GTK test window"
        kill "$gtk_pid" 2>/dev/null || true
        wait "$gtk_pid" 2>/dev/null || true
        cat "$gtk_log" 2>/dev/null || true
        gtk_pid=
        return 1
    fi
    gtk_status=0
    wait "$gtk_pid" || gtk_status=$?
    gtk_pid=
    cat "$gtk_log"
    if [ "$gtk_status" -eq 0 ] \
        && grep -Fx 'pass gtk-ibus-insertion GTK entry contains the committed replacement' \
            "$gtk_log" >/dev/null; then
        log "pass gtk-ibus-insertion-$label GTK received the active IBus engine replacement"
        return 0
    fi
    log "fail gtk-ibus-insertion-$label GTK did not receive the expected IBus replacement"
    return 1
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

# A generic session-bus client must not read focus/layout metadata, switch the
# active input source, or subscribe to focused-window metadata. Check explicit
# authorization errors rather than accepting any failure as evidence.
assert_tracker_method_denied() {
    label=$1
    method=$2
    shift 2
    output="$E2E_WORK/$label.log"
    if gdbus call --session --dest org.gnome.Shell \
        --object-path /org/wayexpand/ShellWindowTracker \
        --method "org.wayexpand.ShellWindowTracker1.$method" "$@" \
        >"$output" 2>&1; then
        log "fail $label a generic session-bus client received tracker access"
        exit 1
    elif grep -qi 'AccessDenied' "$output"; then
        log "pass $label GNOME Shell denied the non-WayExpand peer"
    else
        log "fail $label the request failed without an access-denied result"
        cat "$output"
        exit 1
    fi
}
assert_tracker_method_denied unauthorized-tracker-subscription SubscribeWithTitles true
assert_tracker_method_denied unauthorized-input-source-query GetActiveInputSource
assert_tracker_method_denied unauthorized-keyboard-layout-query GetActiveKeyboardLayout
assert_tracker_method_denied unauthorized-input-source-activation ActivateInputSource xkb de

# Start Shell with an older on-disk identity, then install the current
# metadata while that extension remains loaded. Diagnostics must identify
# the stale in-memory instance as logout-required, not trust the new files.
live_build=$(gdbus call --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.freedesktop.DBus.Properties.Get \
    org.wayexpand.ShellWindowTracker1 BuildIdentity)
case "$live_build" in
    *sha256:pre-upgrade-e2e-fixture*) log "pass extension-pre-upgrade-build $live_build" ;;
    *) log "fail extension-pre-upgrade-build unexpected live identity: $live_build"; exit 1 ;;
esac
cp "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/metadata.json" \
    "$E2E_WORK/home/.local/share/gnome-shell/extensions/$E2E_UUID/metadata.json"
"$E2E_BIN_DIR/wayexpand" doctor --gnome --json \
    >"$E2E_WORK/doctor-after-extension-upgrade.json" \
    2>"$E2E_WORK/doctor-after-extension-upgrade.err" || true
if ! python3 - "$E2E_WORK/doctor-after-extension-upgrade.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    report = json.load(source)
check = next(
    item for item in report["sections"]["window_tracking"]["checks"]
    if item["id"] == "window-extension-build"
)
assert check["state"] in {"warn", "unknown"}, check
print("pass extension-upgrade-not-current updated files are not reported as a verified loaded build")
PY
then
    log "fail extension-upgrade-not-current diagnostics incorrectly reported the updated extension as current"
    cat "$E2E_WORK/doctor-after-extension-upgrade.json"
    exit 1
fi

# Register in GNOME's typed source list without disturbing the keyboard
# layouts. Ask the real CLI to activate WayExpand so the session-bus caller
# authorization and Shell InputSourceManager path are exercised together.
gsettings set org.gnome.desktop.input-sources sources \
    "[('xkb', 'us'), ('xkb', 'de'), ('ibus', 'wayexpand')]"
registered_sources=$(gsettings get org.gnome.desktop.input-sources sources)
case "$registered_sources" in
    *"'ibus', 'wayexpand'"*) log "pass gnome-ibus-source-registered $registered_sources" ;;
    *) log "fail gnome-ibus-source-registered GNOME returned $registered_sources"; exit 1 ;;
esac
# Writing the deprecated `current` key to the WayExpand index must not make
# readiness pass; only Shell's live InputSourceManager can prove activation.
gsettings set org.gnome.desktop.input-sources current 2
"$E2E_BIN_DIR/wayexpand" ibus status --json \
    >"$E2E_WORK/ibus-before-live-activation.json" 2>"$E2E_WORK/ibus-before-live-activation.err" || true
if ! python3 - "$E2E_WORK/ibus-before-live-activation.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as report_file:
    report = json.load(report_file)
check = next(item for item in report["checks"] if item["id"] == "gnome-source-active")
if check["state"] == "pass":
    raise SystemExit("deprecated current readback incorrectly produced an activation PASS")
if check["state"] not in {"warn", "fail", "unknown"}:
    raise SystemExit(f"unexpected activation state: {check['state']!r}")
print(f"pass deprecated-current-no-activation readiness={check['state']}")
PY
then
    log "fail deprecated-current-no-activation CLI accepted the deprecated index as active"
    cat "$E2E_WORK/ibus-before-live-activation.json"
    exit 1
fi

"$E2E_BIN_DIR/wayexpand" doctor --gnome --json \
    >"$E2E_WORK/doctor-before-cli-activation.json" \
    2>"$E2E_WORK/doctor-before-cli-activation.err" || true
if ! python3 - "$E2E_WORK/doctor-before-cli-activation.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    report = json.load(source)
check = next(item for item in report["checks"] if item["id"] == "gnome-source-active")
if check["state"] == "pass":
    raise SystemExit("deprecated current setting incorrectly produced an activation PASS")
print(f"pass deprecated-current-no-activation readiness={check['state']}")
PY
then
    log "fail deprecated-current-no-activation CLI trusted the deprecated setting"
    cat "$E2E_WORK/doctor-before-cli-activation.json"
    exit 1
fi
"$E2E_BIN_DIR/wayexpand" repair --gnome-input-source --json \
    >"$E2E_WORK/ibus-source-activation.json"
"$E2E_BIN_DIR/wayexpand" doctor --gnome --json \
    >"$E2E_WORK/doctor-after-cli-activation.json" 2>&1 || true
python3 - "$E2E_WORK/ibus-source-activation.json" \
    "$E2E_WORK/doctor-after-cli-activation.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    repair = json.load(source)
assert repair.get("state") == "pass", repair
with open(sys.argv[2], encoding="utf-8") as source:
    checks = json.load(source)["checks"]
active = next(item for item in checks if item["id"] == "gnome-source-active")
assert active["state"] == "pass", active
print("pass gnome-ibus-source-active readiness=pass")
PY
# GNOME's active-source manager and IBus engine selection are separate
# observations. Wait for IBus to confirm the selected engine before driving a
# GTK client; this catches activation that only changed Shell's UI state.
tries=100
while :; do
    active_engine=$(ibus engine 2>/dev/null || true)
    case "$active_engine" in
        *wayexpand*)
            log "pass ibus-engine-active $active_engine"
            break
            ;;
    esac
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
        log "fail ibus-engine-active GNOME selected WayExpand but IBus did not confirm its engine: $active_engine"
        exit 1
    fi
    sleep 0.1
done
# Let a real GTK input context receive a trigger from the compositor's
# keyboard path. The direct IBus protocol transaction is covered separately.
if ! run_gtk_insertion before-shell-restart; then
    exit 1
fi
"$E2E_BIN_DIR/wayexpand" ibus status --json \
    >"$E2E_WORK/ibus-after-gtk-insertion.json" \
    2>"$E2E_WORK/ibus-after-gtk-insertion.err" || true
if ! python3 - "$E2E_WORK/ibus-after-gtk-insertion.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    report = json.load(source)
expected = {
    "registered": True,
    "selected": True,
    "engine_running": True,
    "text_received": True,
}
actual = {key: report.get(key) for key in expected}
assert actual == expected, actual
print("pass gnome-ibus-operational the GTK input context reached the selected engine")
PY
then
    log "fail gnome-ibus-operational GTK insertion did not produce healthy IBus engine telemetry"
    cat "$E2E_WORK/ibus-after-gtk-insertion.json"
    exit 1
fi

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

# Restarting Shell in this isolated session models the logout/login needed
# after a GNOME extension update. The newly loaded runtime identity must now
# match the installed files and become ready without changing the binary.
kill "$shell_pid" 2>/dev/null || true
wait "$shell_pid" 2>/dev/null || true
sleep 0.2
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell-after-login.log" 2>&1 &
shell_pid=$!
tries=600
until gnome_extension_state_active "$(extension_state)"; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        log "fail extension-after-login GNOME Shell did not reload the installed extension"
        status=1
        break
    fi
    sleep 0.1
done
if [ "$tries" -gt 0 ]; then
    "$E2E_BIN_DIR/wayexpand" doctor --gnome --json \
        >"$E2E_WORK/doctor-after-login.json" \
        2>"$E2E_WORK/doctor-after-login.err" || true
    if python3 - "$E2E_WORK/doctor-after-login.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    report = json.load(source)
check = next(
    item for item in report["sections"]["window_tracking"]["checks"]
    if item["id"] == "window-extension-build"
)
assert check["state"] == "pass", check
assert "running the installed build" in check["detail"], check
print("pass extension-after-login the reloaded GNOME Shell runs the upgraded extension")
PY
    then
        :
    else
        log "fail extension-after-login installed and loaded extension builds still differ"
        cat "$E2E_WORK/doctor-after-login.json"
        status=1
    fi
fi

# A logout/login is also an input-method lifecycle boundary. Re-select
# WayExpand through GNOME Shell's live manager, then repeat the real GTK
# insertion acceptance test against the newly loaded Shell instance.
if [ "$tries" -gt 0 ]; then
    persisted_sources=$(gsettings get org.gnome.desktop.input-sources sources)
    case "$persisted_sources" in
        *"'ibus', 'wayexpand'"*)
            log "pass gnome-ibus-source-persisted-after-login $persisted_sources"
            ;;
        *)
            log "fail gnome-ibus-source-persisted-after-login GNOME lost the registered source: $persisted_sources"
            status=1
            ;;
    esac

    login_source_ok=0
    if "$E2E_BIN_DIR/wayexpand" repair --gnome-input-source --json \
        >"$E2E_WORK/ibus-source-activation-after-login.json" \
        && { "$E2E_BIN_DIR/wayexpand" doctor --gnome --json \
            >"$E2E_WORK/doctor-source-after-login.json" 2>&1 || true; } \
        && python3 - "$E2E_WORK/ibus-source-activation-after-login.json" \
            "$E2E_WORK/doctor-source-after-login.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    repair = json.load(source)
with open(sys.argv[2], encoding="utf-8") as source:
    checks = json.load(source)["checks"]
active = next(item for item in checks if item["id"] == "gnome-source-active")
assert repair.get("state") == "pass" and active["state"] == "pass", (repair, active)
PY
    then
        login_source_ok=1
        log "pass gnome-ibus-source-after-login live GNOME input source is active"
    else
        log "fail gnome-ibus-source-after-login GNOME could not activate WayExpand after Shell restart"
        status=1
    fi

    if [ "$login_source_ok" -eq 1 ] && ! run_gtk_insertion after-shell-restart; then
        status=1
    fi
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
# Keep WayExpand's private test component while retaining the system IBus
# engines GNOME needs for the configured xkb sources.
env -i \
    HOME="$work/home" \
    PATH="$bin_dir:$PATH" \
    XDG_CONFIG_HOME="$work/config" \
    XDG_CACHE_HOME="$work/cache" \
    XDG_DATA_HOME="$work/home/.local/share" \
    XDG_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" \
    XDG_RUNTIME_DIR="$runtime" \
    IBUS_COMPONENT_PATH="$work/component:/usr/share/ibus/component" \
    GTK_IM_MODULE=ibus \
    GDK_BACKEND=wayland \
    XDG_CURRENT_DESKTOP=GNOME \
    XDG_SESSION_TYPE=wayland \
    E2E_WORK="$work" \
    E2E_BIN_DIR="$bin_dir" \
    E2E_PROJECT_DIR="$project_dir" \
    E2E_UUID="$uuid" \
    E2E_CONTROLLER_UUID="$controller_uuid" \
    E2E_STATE_HELPER="$project_dir/scripts/gnome-extension-state.sh" \
    E2E_DBUS_HELPER="$project_dir/scripts/gnome-extension-dbus.sh" \
    dbus-run-session -- "$work/session.sh" >"$work/results.txt" 2>"$work/session.log" || status=$?

cat "$work/results.txt"
expected_checks='extension-active unauthorized-tracker-subscription extension-pre-upgrade-build extension-upgrade-not-current ibus-engine-registered gnome-ibus-source-registered deprecated-current-no-activation gnome-ibus-source-active gtk-ibus-insertion-before-shell-restart gnome-ibus-operational extension-disabled extension-after-login gnome-ibus-source-persisted-after-login gnome-ibus-source-after-login gtk-ibus-insertion-after-shell-restart'
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
    for log in session.log gnome-shell.log gnome-shell-after-login.log ibus-daemon.log ibus-engines.txt \
        ibus-before-live-activation.json ibus-before-live-activation.err \
        ibus-after-gtk-insertion.json ibus-after-gtk-insertion.err \
        ibus-source-activation-after-login.log \
        unauthorized-tracker-subscription.log \
        doctor-after-extension-upgrade.json \
        doctor-after-extension-upgrade.err doctor-after-login.json \
        gtk-insertion-before-shell-restart.log gtk-insertion-after-shell-restart.log sink.log; do
        printf '%s\n' "--- $log" >&2
        tail -n 60 "$work/$log" >&2 2>/dev/null || true
        if [ -s "$work/$log" ]; then
            log_summary=$(tail -n 12 "$work/$log" | tr '\n' ';' | sed 's/%/%25/g; s/\r/%0D/g')
            printf '::error title=GNOME E2E %s::%s\n' "$log" "$log_summary" >&2
        fi
    done
    exit 1
fi
printf '%s\n' "Headless GNOME input activation, GTK3 IBus insertion, engine health, and extension upgrade/logout tests passed; physical desktop certification remains unverified"
