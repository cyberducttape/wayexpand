#!/bin/sh
# End-to-end test of GNOME source activation, the IBus engine, and window
# tracking in a private headless GNOME Shell session. It verifies GNOME source
# registration/activation, GTK 3/4 input contexts, focus tracking, and
# extension upgrade/reload behavior. The XKB control uses GTK's simple input
# context; IBus cases use the actual IBus module. It does not certify physical
# keyboard behavior.
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
for tool in gnome-shell dbus-run-session gsettings gdbus xdotool xprop xdpyinfo \
    iconv locale xterm timeout python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf '%s\n' "error: $tool is required (install GNOME Shell, XWayland, xdotool and the GTK/IBus test dependencies)" >&2
        exit 2
    fi
done
utf8_charmap=$(LC_ALL=C.UTF-8 locale charmap 2>/dev/null || true)
if [ "$utf8_charmap" != UTF-8 ]; then
    printf '%s\n' "fail gnome-unicode-locale C.UTF-8 locale unavailable (charmap=${utf8_charmap:-unknown})" >&2
    locale -a >&2 2>&1 || true
    exit 1
fi
shell_version=$(gnome-shell --version | sed -n 's/.* \([0-9][0-9]*\)\.\([0-9][0-9]*\).*/\1/p')
if [ -z "$shell_version" ] || ! python3 - "$project_dir/scripts/gnome-e2e-focus-controller/metadata.json" "$shell_version" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    metadata = json.load(source)
if sys.argv[2] not in metadata.get("shell-version", []):
    raise SystemExit(1)
PY
then
    printf '%s\n' "error: the GNOME E2E focus controller does not declare support for GNOME Shell ${shell_version:-unknown}" >&2
    exit 2
fi
browser_bin=${E2E_BROWSER_BIN:-}
if [ -z "$browser_bin" ]; then
    browser_bin=$(command -v google-chrome-stable || command -v google-chrome || true)
fi
if [ -z "$browser_bin" ] || [ ! -x "$browser_bin" ]; then
    printf '%s\n' "error: Google Chrome is required (or set E2E_BROWSER_BIN)" >&2
    exit 2
fi
if ! python3 -c 'import websocket' >/dev/null 2>&1; then
    printf '%s\n' "error: python3-websocket is required for Chromium DevTools verification" >&2
    exit 2
fi

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
    "$project_dir/gnome-extension/$uuid/keyboard-compat.js" \
    "$work/home/.local/share/gnome-shell/extensions/$uuid/"
cp "$project_dir/scripts/gnome-e2e-focus-controller/metadata.json" \
    "$project_dir/scripts/gnome-e2e-focus-controller/extension.js" \
    "$work/home/.local/share/gnome-shell/extensions/$controller_uuid/"
cat >"$work/config/wayexpand/expansions.toml" <<'EOF'
[settings]
compatibility_mode = "tracked-focus"

[[expansion]]
trigger = ";e2e"
replacement = "expanded"

[[expansion]]
trigger = ";unicode"
replacement = "Grüße 世界 🖥️"
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
browser_bin=${E2E_BROWSER_BIN:?E2E_BROWSER_BIN must identify the browser executable}
display=wayexpand-gnome-e2e
log() { printf '%s\n' "$*"; }
diagnose_ibus_session() {
    diagnostic=$1
    {
        printf 'IBUS_COMPONENT_PATH=%s\n' "${IBUS_COMPONENT_PATH:-unset}"
        printf '%s\n' 'Temporary component entries:'
        find "$E2E_WORK/component" -maxdepth 1 -type f -ls 2>&1 || true
        if [ -f "$E2E_WORK/component/wayexpand.xml" ]; then
            printf '%s\n' 'Temporary component XML:'
            cat "$E2E_WORK/component/wayexpand.xml"
        fi
        printf '%s\n' 'System component entry:'
        if [ -f /usr/share/ibus/component/wayexpand.xml ]; then
            ls -l /usr/share/ibus/component/wayexpand.xml
            cat /usr/share/ibus/component/wayexpand.xml
        else
            printf '%s\n' 'not installed'
        fi
        printf '%s\n' 'IBus address:'
        ibus address 2>&1 || true
        printf '%s\n' 'IBus engine registration:'
        ibus list-engine --name-only 2>&1 | grep -Fx wayexpand || true
        printf '%s\n' 'IBus current engine:'
        ibus engine 2>&1 || true
        printf '%s\n' 'GNOME input-source settings:'
        gsettings get org.gnome.desktop.input-sources sources 2>&1 || true
        printf '%s\n' 'IBus daemon PIDs and component/session environment:'
        for pid in $(pgrep -x ibus-daemon 2>/dev/null || true); do
            printf 'pid=%s comm=%s\n' "$pid" "$(cat "/proc/$pid/comm" 2>/dev/null || true)"
            tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null \
                | grep -E '^(IBUS_COMPONENT_PATH|HOME|XDG_RUNTIME_DIR|DBUS_SESSION_BUS_ADDRESS)=' || true
        done
    } >"$diagnostic" 2>&1
}
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
# Where GNOME Tour is installed (Fedora), a fresh profile opens the modal
# WelcomeDialog. It holds the stage grab, so no client receives keyboard input.
gsettings set org.gnome.shell welcome-dialog-last-shown-version 99.0
gsettings set org.gnome.desktop.input-sources sources \
    "[('xkb', 'us'), ('xkb', 'de'), ('xkb', 'fr'), ('ibus', 'wayexpand')]"
# Fedora's xdg-desktop-portal-gnome opens a Remote Desktop dialog in this session
# that nothing can answer, and XTEST input to X11 clients then goes missing.
# Mutter's own RemoteDesktop D-Bus API, which the GTK cases use, is unaffected.
mkdir -p "$XDG_CONFIG_HOME/xdg-desktop-portal"
cat >"$XDG_CONFIG_HOME/xdg-desktop-portal/gnome-portals.conf" <<'PORTALS'
[preferred]
default=gnome;gtk;
org.freedesktop.impl.portal.RemoteDesktop=none
PORTALS
# Let GNOME Shell start and own its normal IBus session. Prestarting an IBus
# daemon in --single mode makes Shell's normal restart path cancel engine
# selection, leaving the source selected in settings but unable to deliver
# key events to GTK.
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell.log" 2>&1 &
shell_pid=$!
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
    # A cold headless Mutter startup can take longer than ten seconds while
    # the renderer initializes. Keep retrying the supported D-Bus activation
    # request for up to a minute before declaring the extension unavailable.
    tries=600
    while ! gnome_shell_extensions_call EnableExtension "$extension_uuid" \
        >"$E2E_WORK/enable-$extension_uuid.log" 2>&1; do
        tries=$((tries - 1))
        if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
            log "fail extension-enable GNOME Shell could not enable $extension_uuid after 60 seconds: $(cat "$E2E_WORK/enable-$extension_uuid.log")"
            exit 1
        fi
        sleep 0.1
    done
done
sinks=
gtk_pid=
browser_pid=
qt_pid=
terminal_pid=
finish() {
    for pid in $sinks; do kill "$pid" 2>/dev/null || true; done
    if [ -n "$gtk_pid" ]; then
        kill "$gtk_pid" 2>/dev/null || true
        wait "$gtk_pid" 2>/dev/null || true
    fi
    if [ -n "$browser_pid" ]; then
        kill "$browser_pid" 2>/dev/null || true
        wait "$browser_pid" 2>/dev/null || true
    fi
    if [ -n "$qt_pid" ]; then
        kill "$qt_pid" 2>/dev/null || true
        wait "$qt_pid" 2>/dev/null || true
    fi
    if [ -n "$terminal_pid" ]; then
        kill "$terminal_pid" 2>/dev/null || true
        wait "$terminal_pid" 2>/dev/null || true
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
        if gdbus call --timeout 5 --session --dest org.gnome.Shell \
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

click_test_field() {
    app_id=$1
    test_window_id=$(DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
        xdotool search --onlyvisible --name "$app_id" 2>/dev/null | tail -n 1)
    if [ -z "$test_window_id" ] || ! DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
        xdotool mousemove --window "$test_window_id" 100 40 click 1; then
        return 1
    fi
}

diagnose_x11_input() {
    case_name=$1
    report="$E2E_WORK/x11-input-$case_name.txt"
    {
        printf 'target_window=%s\n' "${test_window_id:-unknown}"
        focused_window=$(DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xdotool getwindowfocus 2>&1 || true)
        active_window=$(DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xdotool getactivewindow 2>&1 || true)
        printf 'x11_keyboard_focus=%s\n' "$focused_window"
        printf 'x11_active_window=%s\n' "$active_window"
        if [ "$focused_window" = "${test_window_id:-}" ] \
            || [ "$active_window" = "${test_window_id:-}" ]; then
            printf '%s\n' 'x11_test_window_has_focus=yes'
        else
            printf '%s\n' 'x11_test_window_has_focus=no'
            if [ -n "${test_window_id:-}" ]; then
                printf '%s\n' 'attempting targeted windowfocus experiment'
                DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
                    xdotool windowfocus --sync "$test_window_id" 2>&1 || true
                printf 'focus_after_windowfocus=%s\n' \
                    "$(DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" xdotool getwindowfocus 2>&1 || true)"
            fi
        fi
        printf '%s\n' 'focused_window_name:'
        DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xdotool getwindowfocus getwindowname 2>&1 || true
        printf '%s\n' 'active_window_name:'
        DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xdotool getactivewindow getwindowname 2>&1 || true
        if [ -n "${test_window_id:-}" ]; then
            printf '%s\n' 'target_window_properties:'
            DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
                xprop -id "$test_window_id" WM_CLASS _NET_WM_NAME 2>&1 || true
        fi
        printf '%s\n' 'root_active_window:'
        DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xprop -root _NET_ACTIVE_WINDOW 2>&1 || true
        printf '%s\n' 'XTEST extension:'
        DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
            xdpyinfo -queryExtensions 2>&1 | awk 'tolower($0) ~ /xtest/ {print; found=1} END {if (!found) print "not advertised"}'
    } >"$report" 2>&1
}

type_x11_input() {
    case_name=$1
    input_text=$2
    locale_report="$E2E_WORK/locale-$case_name.txt"
    {
        locale
        locale charmap
        locale -a
        printf 'LC_ALL=%s LANG=%s LC_CTYPE=%s\n' \
            "${LC_ALL:-}" "${LANG:-}" "${LC_CTYPE:-}"
        printf 'forced_charmap='
        LC_ALL=C.UTF-8 locale charmap
    } >"$locale_report" 2>&1
    if ! printf '%s' "$input_text" | iconv -f UTF-8 -t UTF-8 >/dev/null; then
        printf '%s\n' 'input is not valid UTF-8' >"$E2E_WORK/xdotool-$case_name.err"
        return 1
    fi
    LC_ALL=C.UTF-8 DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
        xdotool type --clearmodifiers --delay 40 -- "$input_text" \
        2>"$E2E_WORK/xdotool-$case_name.err"
}

# Move libwayland's client trace out of a GTK log into its own file and print
# the events that show whether the client got a keyboard and focus.
report_browser_input_state() {
    x11_focus=$(DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
        xdotool getwindowfocus getwindowname 2>&1 | head -n 1 | cut -c1-100) || true
    log "browser-input-state $1 x11-focus=$x11_focus"
    gdbus call --session --dest org.gnome.Shell \
        --object-path /org/wayexpand/GnomeE2eFocusController \
        --method org.wayexpand.GnomeE2eFocusController.GetShellInputState 2>&1 \
        | cut -c1-700 | sed "s/^/browser-input-state $1 shell=/" || true
}

report_wayland_trace() {
    if [ "$2" != client ]; then
        return 0
    fi
    trace="$1.wayland-trace"
    grep -E '^\[ *[0-9:]+\.[0-9]+\]' "$1" >"$trace" || true
    grep -vE '^\[ *[0-9:]+\.[0-9]+\]' "$1" >"$1.tmp" || true
    mv -f "$1.tmp" "$1"
    log "wayland-trace lines=$(wc -l <"$trace")"
    grep -h 'shell-input-state' "$E2E_WORK/mutter-input-$current_trace_case.txt" 2>/dev/null || true
    grep -E 'wl_seat[@#][0-9]+\.(capabilities|get_keyboard|get_pointer)|wl_keyboard[@#][0-9]+\.(enter|leave|key|keymap)|xdg_toplevel[@#][0-9]+\.configure' \
        "$trace" | cut -c1-120 | head -n 60 | sed 's/^/wayland-trace: /' || true
}

type_gnome_remote_input() {
    case_name=$1
    input_text=$2
    app_id=$3
    focus_marker=$4
    ready_marker=$5
    keycode_probe=${6:-false}
    set --
    if [ "$keycode_probe" = true ]; then
        set -- --keycode-probe
    fi
    python3 "$E2E_PROJECT_DIR/scripts/certification-clients/gnome_remote_input.py" \
        --delay-ms 60 --text "$input_text" --app-id "$app_id" \
        --focus-marker "$focus_marker" --ready-marker "$ready_marker" "$@" \
        >"$E2E_WORK/mutter-input-$case_name.txt" \
        2>"$E2E_WORK/mutter-input-$case_name.err"
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

# Confirm the GNOME-managed IBus daemon loaded the private WayExpand
# component. Checking after Shell starts avoids racing its session startup.
tries=200
while :; do
    if ibus list-engine --name-only >"$E2E_WORK/ibus-engines.txt" 2>&1 \
        && grep -Fx wayexpand "$E2E_WORK/ibus-engines.txt" >/dev/null; then
        log "pass ibus-engine-registered"
        break
    fi
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        diagnose_ibus_session "$E2E_WORK/ibus-registration-diagnostics.txt"
        log "fail ibus-engine-registered GNOME's session IBus registry did not load WayExpand"
        cat "$E2E_WORK/ibus-engines.txt"
        cat "$E2E_WORK/ibus-registration-diagnostics.txt"
        exit 1
    fi
    sleep 0.1
done

# A generic session-bus client must not read focus/layout metadata, switch the
# active input source, or subscribe to focused-window metadata. Check explicit
# authorization errors rather than accepting any failure as evidence.
assert_tracker_method_denied() {
    label=$1
    method=$2
    shift 2
    output="$E2E_WORK/$label.log"
    if gdbus call --timeout 5 --session --dest org.gnome.Shell \
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

# Confirm Shell loaded the bundled build before simulating an on-disk update.
live_build=$(gdbus call --timeout 5 --session --dest org.gnome.Shell \
    --object-path /org/wayexpand/ShellWindowTracker \
    --method org.freedesktop.DBus.Properties.Get \
    org.wayexpand.ShellWindowTracker1 BuildIdentity)
expected_build=$(python3 - "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/metadata.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    print(json.load(source)["x-wayexpand-build-id"])
PY
)
case "$live_build" in
    *"$expected_build"*) log "pass extension-initial-build live build matches the installed files" ;;
    *) log "fail extension-initial-build unexpected live identity: $live_build"; exit 1 ;;
esac

# Register in GNOME's typed source list without disturbing the keyboard
# layouts. Ask the real CLI to activate WayExpand so the session-bus caller
# authorization and Shell InputSourceManager path are exercised together.
gsettings set org.gnome.desktop.input-sources sources \
    "[('xkb', 'us'), ('xkb', 'de'), ('xkb', 'fr'), ('ibus', 'wayexpand')]"
registered_sources=$(gsettings get org.gnome.desktop.input-sources sources)
case "$registered_sources" in
    *"'xkb', 'us'"*"'xkb', 'de'"*"'xkb', 'fr'"*"'ibus', 'wayexpand'"*)
        log "pass gnome-ibus-source-registered $registered_sources"
        ;;
    *)
        log "fail gnome-ibus-source-registered GNOME did not retain the IBus, German, and French sources: $registered_sources"
        exit 1
        ;;
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
    >"$E2E_WORK/doctor-after-cli-activation.json" \
    2>"$E2E_WORK/doctor-after-cli-activation.err" || true
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
# Real GTK text-field checks below are the authoritative GNOME route test.
# The standalone zbus protocol probe is covered by the dedicated real-IBus CI
# job; running it here can perturb older GNOME-managed IBus sessions.
resolve_xwayland_connection() {
    shell_log=$1
    x11_display=$(sed -n 's/.*Using public X11 display \([^,]*\),.*/\1/p' \
        "$shell_log" | head -n 1)
    [ -n "$x11_display" ] || return 1

    # GNOME Shell restarts XWayland during the simulated logout/login. The
    # display number may be reused while its authorization file changes, so
    # resolve a readable auth file for the current XWayland process each time.
    xwayland_auth=
    xwayland_tries=100
    while [ "$xwayland_tries" -gt 0 ]; do
        for process_cmdline in /proc/[0-9]*/cmdline; do
            [ -r "$process_cmdline" ] || continue
            process_args=$(tr '\000' '\n' <"$process_cmdline" 2>/dev/null || true)
            process_display=$(printf '%s\n' "$process_args" | awk '/^:[0-9]+$/ { print; exit }')
            if [ "$process_display" = "$x11_display" ]; then
                candidate_auth=$(printf '%s\n' "$process_args" | awk 'previous == "-auth" { print; exit } { previous = $0 }')
                if [ -n "$candidate_auth" ] && [ -r "$candidate_auth" ]; then
                    xwayland_auth=$candidate_auth
                    return 0
                fi
            fi
        done
        xwayland_tries=$((xwayland_tries - 1))
        sleep 0.1
    done
    return 1
}

if ! resolve_xwayland_connection "$E2E_WORK/gnome-shell.log"; then
    log "fail gnome-gtk-insertion GNOME's private XWayland display or authorization file could not be resolved"
    exit 1
fi
run_gtk_ibus_case() {
    case_name=$1
    trigger=$2
    expected=$3
    password=$4
    gtk_version=${5:-3}
    im_mode=${6:-ibus}
    app_id="org.wayexpand.GnomeGtkIbus.$case_name"
    marker="$E2E_WORK/gtk-window-focused-$case_name"
    ready_marker="$E2E_WORK/gtk-input-ready-$case_name"
    test_log="$E2E_WORK/gtk-ibus-e2e-$case_name.log"
    # Trace the ordinary-input control and the first IBus case (fixed test
    # keys only): whether Mutter delivers keys decides everything after them.
    wayland_debug=0
    current_trace_case=$case_name
    case "$case_name" in
        wayland-baseline | insertion) wayland_debug=client ;;
    esac
    engine_log="$E2E_WORK/ibus-engine-$case_name.txt"
    status_file="$XDG_RUNTIME_DIR/wayexpand-ibus-status.json"
    printf 'before_gtk=' >"$engine_log"
    ibus engine >>"$engine_log" 2>&1 || true
    set --
    if [ "$password" = true ]; then
        set -- --password
    fi
    if [ "$im_mode" = simple ]; then
        # Force GTK's built-in simple context so this XKB control cannot
        # attach to IBus. The following cases test the IBus module explicitly.
        timeout 20s env -u DISPLAY -u XAUTHORITY -u XMODIFIERS -u IBUS_ADDRESS \
            GTK_IM_MODULE=gtk-im-context-simple \
            WAYLAND_DEBUG="$wayland_debug" \
            GDK_BACKEND=wayland \
            GTK_E2E_FOCUS_MARKER="$marker" \
            GTK_E2E_READY_MARKER="$ready_marker" \
            GTK_E2E_APP_ID="$app_id" \
            python3 "$E2E_PROJECT_DIR/scripts/certification-clients/gtk_ibus_e2e.py" \
            --gtk-version "$gtk_version" --im-mode simple --expected "$expected" "$@" >"$test_log" 2>&1 &
    else
        timeout 20s env -u DISPLAY -u XAUTHORITY \
            GTK_IM_MODULE=ibus \
            WAYLAND_DEBUG="$wayland_debug" \
            GDK_BACKEND=wayland \
            IBUS_ADDRESS="$(ibus address)" \
            GTK_E2E_FOCUS_MARKER="$marker" \
            GTK_E2E_READY_MARKER="$ready_marker" \
            GTK_E2E_APP_ID="$app_id" \
            python3 "$E2E_PROJECT_DIR/scripts/certification-clients/gtk_ibus_e2e.py" \
            --gtk-version "$gtk_version" --im-mode ibus --expected "$expected" "$@" >"$test_log" 2>&1 &
    fi
    gtk_pid=$!
    if [ -f "$status_file" ]; then
        cp "$status_file" "$E2E_WORK/ibus-context-before-$case_name.json"
    fi
    input_status=0
    keycode_probe=false
    if [ "$case_name" = wayland-baseline ]; then
        keycode_probe=true
    fi
    type_gnome_remote_input "$case_name" "$trigger" "$app_id" \
        "$marker" "$ready_marker" "$keycode_probe" || input_status=$?
    if [ "$input_status" -ne 0 ]; then
        if [ -f "$status_file" ]; then
            cp "$status_file" "$E2E_WORK/ibus-context-after-$case_name.json"
        fi
        kill "$gtk_pid" 2>/dev/null || true
        wait "$gtk_pid" 2>/dev/null || true
        gtk_pid=
        report_wayland_trace "$test_log" "$wayland_debug"
        if grep -F 'GTK did not confirm stable active input focus' \
            "$E2E_WORK/mutter-input-$case_name.err" >/dev/null; then
            log "fail gnome-gtk-$case_name GTK never acquired active keyboard focus; no test input was sent"
            cat "$test_log" "$E2E_WORK/mutter-input-$case_name.err"
        elif [ "$case_name" = wayland-baseline ]; then
            log "fail gnome-gtk-$case_name Mutter RemoteDesktop could not send a key to the focused GTK window"
            cat "$test_log" "$E2E_WORK/mutter-input-$case_name.txt" \
                "$E2E_WORK/mutter-input-$case_name.err"
        else
            log "fail gnome-gtk-$case_name Mutter RemoteDesktop could not type into the focused GTK window"
            cat "$test_log" "$E2E_WORK/mutter-input-$case_name.txt" \
                "$E2E_WORK/mutter-input-$case_name.err"
        fi
        exit 1
    fi
    if [ -f "$status_file" ]; then
        cp "$status_file" "$E2E_WORK/ibus-context-after-$case_name.json"
    fi
    gtk_status=0
    wait "$gtk_pid" || gtk_status=$?
    gtk_pid=
    report_wayland_trace "$test_log" "$wayland_debug"
    printf 'after_gtk=' >>"$engine_log"
    ibus engine >>"$engine_log" 2>&1 || true
    if [ "$gtk_status" -ne 0 ]; then
        cp "$status_file" "$E2E_WORK/ibus-status-$case_name.json" 2>/dev/null || true
        if [ "$im_mode" = simple ]; then
            log "fail gnome-gtk-$case_name GNOME_INPUT_NOT_DELIVERED: ordinary XKB input did not reach the GTK field"
        else
            log "fail gnome-gtk-$case_name GTK's IBus input context produced an incorrect result"
        fi
        cat "$test_log"
        cat "$engine_log"
        cat "$E2E_WORK/mutter-input-$case_name.txt" \
            "$E2E_WORK/mutter-input-$case_name.err" 2>/dev/null || true
        python3 - "$E2E_WORK/ibus-context-before-$case_name.json" \
            "$E2E_WORK/ibus-context-after-$case_name.json" <<'PY'
import json
import pathlib
import sys

for filename in sys.argv[1:]:
    path = pathlib.Path(filename)
    if not path.is_file():
        continue
    status = json.loads(path.read_text(encoding="utf-8"))
    print(path.name, json.dumps({
        "totals": status.get("totals"),
        "recent_fields": [
            {key: field.get(key) for key in
             ("sequence", "focused", "keys", "expansions", "content_type")}
            for field in status.get("recent_fields", [])
        ],
    }, sort_keys=True))
PY
        if [ -s "$E2E_WORK/ibus-status-$case_name.json" ]; then
            python3 - "$E2E_WORK/ibus-status-$case_name.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as status_file:
    status = json.load(status_file)
print("content-free IBus counters:", json.dumps({
    "totals": status.get("totals"),
    "recent_fields": status.get("recent_fields"),
}, sort_keys=True))
PY
        fi
        exit 1
    fi
    log "pass gnome-gtk-$case_name $(cat "$test_log")"
}

run_terminal_x11_baseline() {
    case_name=x11-baseline
    input=a
    app_id=org.wayexpand.GnomeTerminalX11Baseline
    ready_marker="$E2E_WORK/terminal-ready-$case_name"
    test_log="$E2E_WORK/terminal-ibus-e2e-$case_name.log"
    timeout 20s env -u GTK_IM_MODULE -u XMODIFIERS -u IBUS_ADDRESS \
        DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
        xterm -title "WayExpand terminal acceptance $app_id" \
        -geometry 80x3+0+0 -e python3 \
        "$E2E_PROJECT_DIR/scripts/certification-clients/terminal_ibus_e2e.py" \
        --expected "$input" --ready-marker "$ready_marker" \
        >"$test_log" 2>&1 &
    terminal_pid=$!
    if ! activate_window "$app_id" || ! click_test_field "$app_id"; then
        kill "$terminal_pid" 2>/dev/null || true
        wait "$terminal_pid" 2>/dev/null || true
        terminal_pid=
        log "fail gnome-x11-terminal-baseline could not focus the basic X11 terminal reader"
        cat "$test_log"
        exit 1
    fi
    ready_tries=100
    while [ ! -s "$ready_marker" ]; do
        if ! kill -0 "$terminal_pid" 2>/dev/null; then
            wait "$terminal_pid" || true
            terminal_pid=
            log "fail gnome-x11-terminal-baseline basic X11 terminal reader exited before readiness"
            cat "$test_log"
            exit 1
        fi
        ready_tries=$((ready_tries - 1))
        if [ "$ready_tries" -le 0 ]; then
            kill "$terminal_pid" 2>/dev/null || true
            wait "$terminal_pid" 2>/dev/null || true
            terminal_pid=
            log "fail gnome-x11-terminal-baseline basic X11 terminal reader did not become ready"
            cat "$test_log"
            exit 1
        fi
        sleep 0.05
    done
    diagnose_x11_input "$case_name"
    if ! type_x11_input "$case_name" "$input"; then
        kill "$terminal_pid" 2>/dev/null || true
        wait "$terminal_pid" 2>/dev/null || true
        terminal_pid=
        log "fail gnome-x11-terminal-baseline XTEST could not send the ordinary test key"
        cat "$test_log" "$E2E_WORK/x11-input-$case_name.txt" \
            "$E2E_WORK/locale-$case_name.txt" "$E2E_WORK/xdotool-$case_name.err"
        exit 1
    fi
    terminal_status=0
    wait "$terminal_pid" || terminal_status=$?
    terminal_pid=
    if [ "$terminal_status" -ne 0 ]; then
        log "fail gnome-x11-terminal-baseline XTEST_INPUT_NOT_DELIVERED: key did not reach a basic X11 application"
        cat "$test_log" "$E2E_WORK/x11-input-$case_name.txt" \
            "$E2E_WORK/locale-$case_name.txt" "$E2E_WORK/xdotool-$case_name.err"
        exit 1
    fi
    log "pass gnome-x11-terminal-baseline $(cat "$test_log")"
}

# Test GTK's native Wayland input path separately from WayExpand's IBus engine.
# The baseline uses the US XKB source and GtkIMContextSimple; terminal X11/XTEST
# input is exercised independently above.
if ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --activate-source xkb us \
    >"$E2E_WORK/layout-us-baseline-activation.log" 2>&1; then
    log "fail gnome-gtk-wayland-baseline GNOME could not activate the ordinary US XKB source"
    cat "$E2E_WORK/layout-us-baseline-activation.log"
    exit 1
fi
if ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --active-layout \
    >"$E2E_WORK/layout-us-baseline-active.log" 2>&1 \
    || ! grep -F 'active-layout xkb us ' "$E2E_WORK/layout-us-baseline-active.log" >/dev/null; then
    log "fail gnome-gtk-wayland-baseline the ordinary US XKB source is not active"
    cat "$E2E_WORK/layout-us-baseline-activation.log" "$E2E_WORK/layout-us-baseline-active.log"
    exit 1
fi
run_terminal_x11_baseline
run_gtk_ibus_case wayland-baseline a a false 3 simple
if ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --activate-source ibus wayexpand \
    >"$E2E_WORK/ibus-source-after-x11-baseline.log" 2>&1; then
    log "fail ibus-engine-active GNOME could not restore WayExpand after the XKB baseline"
    cat "$E2E_WORK/ibus-source-after-x11-baseline.log"
    exit 1
fi
tries=100
while :; do
    active_engine=$(ibus engine 2>/dev/null || true)
    case "$active_engine" in
        *wayexpand*)
            log "pass ibus-engine-active WayExpand restored after XKB baseline"
            break
            ;;
    esac
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
        log "fail ibus-engine-active WayExpand did not resume after XKB baseline: $active_engine"
        exit 1
    fi
    sleep 0.1
done

run_gtk_ibus_case insertion ';e2e' expanded false
run_gtk_ibus_case unicode ';unicode' 'Grüße 世界 🖥️' false
run_gtk_ibus_case gtk4-insertion ';e2e' expanded false 4
run_gtk_ibus_case gtk4-unicode ';unicode' 'Grüße 世界 🖥️' false 4
run_qt_xim_case() {
    case_name=$1
    input=$2
    expected=$3
    password=$4
    app_id="org.wayexpand.GnomeQtIbus.$case_name"
    marker="$E2E_WORK/qt-window-focused-$case_name"
    ready_marker="$E2E_WORK/qt-input-ready-$case_name"
    test_log="$E2E_WORK/qt-ibus-e2e-$case_name.log"
    set --
    if [ "$password" = true ]; then
        set -- --password
    fi
    timeout 20s env \
        QT_QPA_PLATFORM=xcb \
        QT_IM_MODULE=xim \
        XMODIFIERS='@im=ibus' \
        DISPLAY="$x11_display" \
        XAUTHORITY="$xwayland_auth" \
        IBUS_ADDRESS="$(ibus address)" \
        QT_E2E_FOCUS_MARKER="$marker" \
        QT_E2E_READY_MARKER="$ready_marker" \
        QT_E2E_APP_ID="$app_id" \
        python3 "$E2E_PROJECT_DIR/scripts/certification-clients/qt_ibus_e2e.py" \
        --expected "$expected" "$@" >"$test_log" 2>&1 &
    qt_pid=$!
    if ! activate_window "$app_id"; then
        kill "$qt_pid" 2>/dev/null || true
        wait "$qt_pid" 2>/dev/null || true
        log "fail gnome-qt-$case_name Mutter could not focus the Qt XIM acceptance window"
        cat "$test_log"
        exit 1
    fi
    if ! click_test_field "$app_id"; then
        kill "$qt_pid" 2>/dev/null || true
        wait "$qt_pid" 2>/dev/null || true
        qt_pid=
        log "fail gnome-qt-$case_name XTEST could not focus the target field"
        cat "$test_log"
        exit 1
    fi
    : >"$marker"
    ready_tries=100
    while [ ! -s "$ready_marker" ]; do
        if ! kill -0 "$qt_pid" 2>/dev/null; then
            wait "$qt_pid" || true
            qt_pid=
            log "fail gnome-qt-$case_name Qt input context exited before readiness"
            cat "$test_log"
            exit 1
        fi
        ready_tries=$((ready_tries - 1))
        if [ "$ready_tries" -le 0 ]; then
            kill "$qt_pid" 2>/dev/null || true
            wait "$qt_pid" 2>/dev/null || true
            qt_pid=
            log "fail gnome-qt-$case_name Qt field did not report stable active focus"
            cat "$test_log"
            exit 1
        fi
        sleep 0.05
    done
    diagnose_x11_input "$case_name"
    if ! type_x11_input "$case_name" "$input"; then
        kill "$qt_pid" 2>/dev/null || true
        wait "$qt_pid" 2>/dev/null || true
        log "fail gnome-qt-$case_name XTEST could not type into the Qt field"
        cat "$test_log" "$E2E_WORK/x11-input-$case_name.txt" \
            "$E2E_WORK/locale-$case_name.txt" "$E2E_WORK/xdotool-$case_name.err"
        exit 1
    fi
    qt_status=0
    wait "$qt_pid" || qt_status=$?
    qt_pid=
    if [ "$qt_status" -ne 0 ]; then
        log "fail gnome-qt-$case_name Qt XIM changed or lost input"
        cat "$test_log"
        exit 1
    fi
    log "pass gnome-qt-$case_name $(cat "$test_log")"
}
run_terminal_xim_case() {
    case_name=$1
    input=$2
    ready_marker="$E2E_WORK/terminal-ready-$case_name"
    test_log="$E2E_WORK/terminal-ibus-e2e-$case_name.log"
    DISPLAY="$x11_display" \
        XAUTHORITY="$xwayland_auth" \
        IBUS_ADDRESS="$(ibus address)" \
        XMODIFIERS='@im=ibus' \
        xterm -title "WayExpand terminal acceptance org.wayexpand.GnomeTerminalIbus" \
        -geometry 80x3+0+0 -e python3 \
        "$E2E_PROJECT_DIR/scripts/certification-clients/terminal_ibus_e2e.py" \
        --expected "$input" --ready-marker "$ready_marker" \
        >"$test_log" 2>&1 &
    terminal_pid=$!
    ready_tries=100
    while [ ! -s "$ready_marker" ]; do
        if ! kill -0 "$terminal_pid" 2>/dev/null; then
            wait "$terminal_pid" || true
            terminal_pid=
            log "fail gnome-terminal-$case_name terminal input reader exited before readiness"
            cat "$test_log"
            exit 1
        fi
        ready_tries=$((ready_tries - 1))
        if [ "$ready_tries" -le 0 ]; then
            kill "$terminal_pid" 2>/dev/null || true
            wait "$terminal_pid" 2>/dev/null || true
            terminal_pid=
            log "fail gnome-terminal-$case_name terminal input reader did not become ready"
            cat "$test_log"
            exit 1
        fi
        sleep 0.05
    done
    if ! activate_window org.wayexpand.GnomeTerminalIbus; then
        kill "$terminal_pid" 2>/dev/null || true
        wait "$terminal_pid" 2>/dev/null || true
        terminal_pid=
        log "fail gnome-terminal-$case_name Mutter could not focus the terminal acceptance window"
        cat "$test_log"
        exit 1
    fi
    if ! type_x11_input "$case_name" "$input"; then
        kill "$terminal_pid" 2>/dev/null || true
        wait "$terminal_pid" 2>/dev/null || true
        terminal_pid=
        log "fail gnome-terminal-$case_name XTEST could not type into the terminal"
        cat "$test_log" "$E2E_WORK/locale-$case_name.txt" \
            "$E2E_WORK/xdotool-$case_name.err"
        exit 1
    fi
    terminal_status=0
    wait "$terminal_pid" || terminal_status=$?
    terminal_pid=
    if [ "$terminal_status" -ne 0 ]; then
        log "fail gnome-terminal-$case_name terminal XIM changed or lost input"
        cat "$test_log"
        exit 1
    fi
    log "pass gnome-terminal-$case_name $(cat "$test_log")"
}
for layout in de fr; do
    "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --activate-source xkb "$layout" \
        >"$E2E_WORK/layout-$layout-activation.log"
    if ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --active-layout \
        >"$E2E_WORK/layout-$layout-active.log" \
        || ! grep -F "active-layout xkb $layout " "$E2E_WORK/layout-$layout-active.log" >/dev/null; then
        log "fail gnome-layout-$layout GNOME did not report the selected keyboard layout"
        cat "$E2E_WORK/layout-$layout-activation.log" "$E2E_WORK/layout-$layout-active.log"
        exit 1
    fi
    activation_tries=50
    while ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --activate-source ibus wayexpand \
        >"$E2E_WORK/layout-$layout-ibus-activation.log" 2>&1; do
        activation_tries=$((activation_tries - 1))
        if [ "$activation_tries" -le 0 ]; then
            diagnostic="$E2E_WORK/layout-$layout-ibus-state.txt"
            diagnose_ibus_session "$diagnostic"
            log "fail gnome-layout-$layout GNOME did not restore WayExpand after a layout switch"
            cat "$E2E_WORK/layout-$layout-ibus-activation.log" "$diagnostic"
            exit 1
        fi
        sleep 0.1
    done
    run_gtk_ibus_case "layout-$layout" ';e2e' expanded false
    log "pass gnome-layout-$layout expansion remained correct after switching layouts"
done
# Exercise the same active engine in a real browser renderer. The isolated
# Chromium profile uses a local page, XTEST sends only the harmless `;e2e`
# trigger, and DevTools reads only that test textarea's final value.
env \
    GTK_IM_MODULE=ibus \
    DISPLAY="$x11_display" \
    XAUTHORITY="$xwayland_auth" \
    IBUS_ADDRESS="$(ibus address)" \
    E2E_BROWSER_BIN="$browser_bin" \
    python3 "$E2E_PROJECT_DIR/scripts/certification-clients/browser_ibus_e2e.py" \
    --browser "$browser_bin" \
    --profile "$E2E_WORK/browser-profile" \
    --page "$E2E_WORK/browser-ibus-e2e.html" \
    --ready-marker "$E2E_WORK/browser-window-ready" \
    --focus-marker "$E2E_WORK/browser-window-focused" \
    --armed-marker "$E2E_WORK/browser-window-armed" \
    --status-file "$XDG_RUNTIME_DIR/wayexpand-ibus-status.json" \
    --log "$E2E_WORK/browser-ibus-e2e-chrome.log" \
    >"$E2E_WORK/browser-ibus-e2e.log" 2>&1 &
browser_pid=$!
# Give Chromium's independent DevTools readiness deadline room to report its
# failure details instead of racing this wrapper's timeout and being killed.
tries=600
while [ ! -e "$E2E_WORK/browser-window-ready" ]; do
    if ! kill -0 "$browser_pid" 2>/dev/null; then
        wait "$browser_pid" || true
        browser_pid=
        log "fail gnome-browser-insertion Chromium could not open the isolated acceptance page"
        cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
        exit 1
    fi
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
        kill "$browser_pid" 2>/dev/null || true
        wait "$browser_pid" 2>/dev/null || true
        browser_pid=
        log "fail gnome-browser-insertion Chromium did not become ready"
        cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
        exit 1
    fi
    sleep 0.1
done
if ! activate_window org.wayexpand.GnomeBrowserIbus; then
    kill "$browser_pid" 2>/dev/null || true
    wait "$browser_pid" 2>/dev/null || true
    browser_pid=
    log "fail gnome-browser-insertion Mutter could not focus Chromium's acceptance window"
    cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
    exit 1
fi
: >"$E2E_WORK/browser-window-focused"
tries=200
while [ ! -e "$E2E_WORK/browser-window-armed" ]; do
    if ! kill -0 "$browser_pid" 2>/dev/null; then
        wait "$browser_pid" || true
        browser_pid=
        log "fail gnome-browser-insertion Chromium did not arm its correlated input check"
        cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
        exit 1
    fi
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
        kill "$browser_pid" 2>/dev/null || true
        wait "$browser_pid" 2>/dev/null || true
        browser_pid=
        log "fail gnome-browser-insertion Chromium did not arm its correlated input check"
        cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
        exit 1
    fi
    sleep 0.1
done
report_browser_input_state before-typing
if ! DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \
    xdotool type --clearmodifiers --delay 40 -- ';e2e'; then
    kill "$browser_pid" 2>/dev/null || true
    wait "$browser_pid" 2>/dev/null || true
    browser_pid=
    log "fail gnome-browser-insertion XTEST could not type into the focused browser window"
    cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
    exit 1
fi
report_browser_input_state after-typing
browser_status=0
wait "$browser_pid" || browser_status=$?
browser_pid=
if [ "$browser_status" -ne 0 ]; then
    log "fail gnome-browser-insertion Chromium's IBus text field did not receive the replacement"
    cat "$E2E_WORK/browser-ibus-e2e.log" "$E2E_WORK/browser-ibus-e2e-chrome.log"
    exit 1
fi
cat "$E2E_WORK/browser-ibus-e2e.log"
if grep -F 'Chromium IBus input context replaced the test trigger' \
    "$E2E_WORK/browser-ibus-e2e.log" >/dev/null; then
    log "pass gnome-browser-insertion Chromium's IBus input context replaced the trigger in a textarea"
elif grep -F 'Chromium input reached WayExpand; without IBus surrounding-text support, all trigger keys passed through unchanged' \
    "$E2E_WORK/browser-ibus-e2e.log" >/dev/null; then
    log "pass gnome-browser-safe-passthrough Chromium lacks IBus surrounding-text support; all trigger keys reached the field unchanged"
else
    log "fail gnome-browser-insertion browser result did not prove insertion or safe pass-through"
    cat "$E2E_WORK/browser-ibus-e2e.log"
    exit 1
fi
"$E2E_BIN_DIR/wayexpand" ibus status --json \
    >"$E2E_WORK/ibus-after-input-context.json" \
    2>"$E2E_WORK/ibus-after-input-context.err" || true
if ! python3 - "$E2E_WORK/ibus-after-input-context.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    report = json.load(source)
readiness = report.get("readiness", {})
expected = {
    "registered": True,
    "selected": True,
    "engine_running": True,
    "text_received": True,
}
actual = {key: readiness.get(key) for key in expected}
assert actual == expected, actual
print("pass gnome-ibus-input-context the live GNOME engine replaced a verified trigger")
PY
then
    log "fail gnome-ibus-input-context the live engine did not report healthy IBus telemetry"
    cat "$E2E_WORK/ibus-after-input-context.json"
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

# Simulate an extension upgrade on disk while Shell still has the previous
# build loaded. Diagnostics must report the mismatch until Shell restarts.
python3 - "$E2E_WORK/home/.local/share/gnome-shell/extensions/$E2E_UUID/metadata.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as source:
    metadata = json.load(source)
metadata["x-wayexpand-build-id"] = "sha256:post-upgrade-e2e-fixture"
with open(path, "w", encoding="utf-8") as destination:
    json.dump(metadata, destination)
    destination.write("\n")
PY
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

# The updated payload is the currently bundled build in this test checkout.
# Restore it before restarting Shell so the new process loads that build.
cp "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/metadata.json" \
    "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/extension.js" \
    "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/notification-queue.js" \
    "$E2E_PROJECT_DIR/gnome-extension/$E2E_UUID/keyboard-compat.js" \
    "$E2E_WORK/home/.local/share/gnome-shell/extensions/$E2E_UUID/"

# Restarting Shell in this isolated session models the logout/login needed
# after a GNOME extension update. The newly loaded runtime identity must now
# match the installed files and become ready without changing the binary.
kill "$shell_pid" 2>/dev/null || true
wait "$shell_pid" 2>/dev/null || true
sleep 0.2
gnome-shell --headless --mode=user --wayland --virtual-monitor 1280x800 --wayland-display "$display" \
    >"$E2E_WORK/gnome-shell-after-login.log" 2>&1 &
shell_pid=$!
# GNOME Shell may retain the enabled-extensions preference across restart
# without instantiating an extension in this headless session. Ask the live
# extension manager to load the installed extension before checking its
# runtime identity.
enable_tries=100
until gnome_shell_extensions_call EnableExtension "$E2E_UUID" \
    >"$E2E_WORK/enable-after-login.log" 2>&1; do
    enable_tries=$((enable_tries - 1))
    if [ "$enable_tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        log "fail extension-after-login GNOME Shell could not enable the installed extension: $(cat "$E2E_WORK/enable-after-login.log")"
        exit 1
    fi
    sleep 0.1
done
tries=600
until [ "$enable_tries" -le 0 ] || gnome_extension_state_active "$(extension_state)"; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
        log "fail extension-after-login GNOME Shell did not reload the installed extension"
        status=1
        break
    fi
    sleep 0.1
done
if [ "$tries" -gt 0 ]; then
    # GNOME can report an extension active before its D-Bus object has been
    # exported. Wait for the live identity itself, which proves that the new
    # Shell instance loaded and registered the updated implementation.
    live_build=
    build_tries=100
    while [ "$build_tries" -gt 0 ]; do
        live_build=$(gdbus call --timeout 5 --session --dest org.gnome.Shell \
            --object-path /org/wayexpand/ShellWindowTracker \
            --method org.freedesktop.DBus.Properties.Get \
            org.wayexpand.ShellWindowTracker1 BuildIdentity 2>/dev/null || true)
        case "$live_build" in
            *"$expected_build"*) break ;;
        esac
        build_tries=$((build_tries - 1))
        sleep 0.1
    done
    # The extension's build property can become available just before its
    # tracker handshake is ready. Wait for the complete doctor result instead
    # of treating that brief startup window as a failed logout/login test.
    readiness_tries=100
    readiness_ok=0
    while [ "$build_tries" -gt 0 ] && [ "$readiness_tries" -gt 0 ]; do
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
PY
        then
            readiness_ok=1
            break
        fi
        readiness_tries=$((readiness_tries - 1))
        sleep 0.1
    done
    if [ "$build_tries" -gt 0 ] && [ "$readiness_ok" -eq 1 ]; then
        log "pass extension-after-login the reloaded GNOME Shell runs the upgraded extension"
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
            >"$E2E_WORK/doctor-source-after-login.json" \
            2>"$E2E_WORK/doctor-source-after-login.err" || true; } \
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

    if [ "$login_source_ok" -eq 1 ]; then
        if ! resolve_xwayland_connection "$E2E_WORK/gnome-shell-after-login.log"; then
            log "fail gnome-x11-after-login the reloaded session's XWayland display or authorization file could not be resolved"
            status=1
        else
            run_gtk_ibus_case after-login ';e2e' expanded false 3
            log "pass gnome-gtk-after-login the reloaded session replaced a verified trigger in GTK"
        fi
    fi
fi

# Qt and xterm use IBus XIM. On Ubuntu, the first XIM client can cause the
# session to start/restart ibus-daemon with --xim. Keep those clients after
# GNOME source switching, GTK replacement, and logout/relogin assertions so
# their compatibility setup cannot invalidate the GNOME-managed engine under
# test. These checks assert exact pass-through, never replacement support.
"$E2E_BIN_DIR/wx-e2e-gnome-tracker" --activate-source xkb de \
    >"$E2E_WORK/layout-de-xim-activation.log"
if ! "$E2E_BIN_DIR/wx-e2e-gnome-tracker" --active-layout \
    >"$E2E_WORK/layout-de-xim-active.log" \
    || ! grep -F 'active-layout xkb de ' "$E2E_WORK/layout-de-xim-active.log" >/dev/null; then
    log "fail gnome-layout-de-xim German layout was not active for Unicode XIM input"
    cat "$E2E_WORK/layout-de-xim-activation.log" "$E2E_WORK/layout-de-xim-active.log"
    exit 1
fi
run_qt_xim_case safe-passthrough ';e2e' ';e2e' false
run_terminal_xim_case safe-passthrough ';e2e'
# XTEST can only type symbols present in the active XKB keymap. GTK's IBus
# commit cases above cover broader CJK and emoji output.
run_qt_xim_case unicode-passthrough 'Grüße' 'Grüße' false
run_terminal_xim_case unicode-passthrough 'Grüße'

# Password-purpose IBus contexts intentionally tell GNOME to disable IBus for
# sensitive fields. Run these checks after every scenario that requires the
# IBus source, because closing a password field does not itself report a new
# normal-purpose context to GNOME. This preserves the privacy check without
# leaving the remainder of the suite in GNOME's temporary IBus-disabled mode.
run_qt_xim_case password ';e2e' ';e2e' true
run_gtk_ibus_case password ';e2e' ';e2e' true
run_gtk_ibus_case gtk4-password ';e2e' ';e2e' true 4

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
    WAYLAND_DISPLAY=wayexpand-gnome-e2e \
    IBUS_COMPONENT_PATH="${E2E_IBUS_COMPONENT_PATH:-$work/component:/usr/share/ibus/component}" \
    GTK_IM_MODULE=ibus \
    GDK_BACKEND=wayland \
    XDG_CURRENT_DESKTOP=GNOME \
    XDG_SESSION_TYPE=wayland \
    WAYEXPAND_GNOME_AUTH_DIAGNOSTICS=1 \
    WAYEXPAND_GNOME_TEST_CLIENTS="$bin_dir/wayexpand:$bin_dir/wayexpand-ibus:$bin_dir/wx-e2e-gnome-tracker" \
    E2E_WORK="$work" \
    E2E_BIN_DIR="$bin_dir" \
    E2E_BROWSER_BIN="$browser_bin" \
    E2E_BROWSER_NO_SANDBOX="${E2E_BROWSER_NO_SANDBOX:-0}" \
    E2E_PROJECT_DIR="$project_dir" \
    E2E_UUID="$uuid" \
    E2E_CONTROLLER_UUID="$controller_uuid" \
    E2E_STATE_HELPER="$project_dir/scripts/gnome-extension-state.sh" \
    E2E_DBUS_HELPER="$project_dir/scripts/gnome-extension-dbus.sh" \
    dbus-run-session -- "$work/session.sh" >"$work/results.txt" 2>"$work/session.log" || status=$?

cat "$work/results.txt"
expected_checks='extension-active unauthorized-tracker-subscription extension-initial-build extension-upgrade-not-current ibus-engine-registered gnome-ibus-source-registered deprecated-current-no-activation gnome-ibus-source-active gnome-x11-terminal-baseline gnome-gtk-wayland-baseline gnome-gtk-insertion gnome-gtk-unicode gnome-gtk-password gnome-gtk-gtk4-insertion gnome-gtk-gtk4-unicode gnome-gtk-gtk4-password gnome-qt-safe-passthrough gnome-qt-unicode-passthrough gnome-qt-password gnome-terminal-safe-passthrough gnome-terminal-unicode-passthrough gnome-layout-de gnome-layout-fr extension-disabled extension-after-login gnome-ibus-source-persisted-after-login gnome-ibus-source-after-login gnome-gtk-after-login'
missing_check=
for check in $expected_checks; do
    if ! grep -F "pass $check" "$work/results.txt" >/dev/null; then
        missing_check=$check
        break
    fi
done
if ! grep -Eq '^pass gnome-browser-(insertion|safe-passthrough)' "$work/results.txt"; then
    missing_check=${missing_check:-gnome-browser-result}
fi
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
        ibus-registration-diagnostics.txt \
        ibus-before-live-activation.json ibus-before-live-activation.err \
        ibus-after-input-context.json ibus-after-input-context.err \
        ibus-source-activation-after-login.log layout-de-ibus-state.txt \
        layout-fr-ibus-state.txt \
        browser-ibus-e2e.log browser-ibus-e2e-chrome.log \
        unauthorized-tracker-subscription.log \
        doctor-after-extension-upgrade.json \
        doctor-after-extension-upgrade.err doctor-after-login.json \
        sink.log; do
        printf '%s\n' "--- $log" >&2
        tail -n 60 "$work/$log" >&2 2>/dev/null || true
        if [ -s "$work/$log" ]; then
            log_summary=$(tail -n 12 "$work/$log" | tr '\n' ';' | sed 's/%/%25/g; s/\r/%0D/g')
            printf '::error title=GNOME E2E %s::%s\n' "$log" "$log_summary" >&2
        fi
    done
    exit 1
fi
printf '%s\n' "Headless GNOME input activation, GTK3/GTK4 replacement/Unicode/password-field behavior, Qt and terminal safe pass-through, Chromium IBus fail-safe behavior, engine health, layout switching, and extension upgrade/logout tests passed; Firefox, Electron, package upgrade/uninstall persistence, and physical desktop certification remain unverified"
