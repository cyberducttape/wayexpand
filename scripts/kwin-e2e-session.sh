#!/bin/sh
# Session driver for the headless KWin end-to-end harness.
#
# Runs inside the private session that scripts/test-kwin-e2e.sh starts
# (dbus-run-session + kwin_wayland --virtual); never run it in a real
# desktop session. The scenarios live in scripts/e2e-lib.sh. Here the
# daemon uses:
#   - capture: --source=stdin (the harness feeds what the "user" typed;
#     real keyboard capture needs /dev/input and is not exercised here),
#   - output: the production libei backend, reaching KWin's EIS through
#     wx-e2e-eis-relay instead of the RemoteDesktop portal,
#   - focus: the production KWin window tracker on the private bus, with
#     compatibility_mode = "tracked-focus".
# The "user" types into the focused sink with wx-e2e-type (also libei).
set -u

# shellcheck source=scripts/e2e-lib.sh
. "$(dirname -- "$0")/e2e-lib.sh"

export LIBEI_SOCKET="$XDG_RUNTIME_DIR/eis.sock"
export WX_E2E_TYPIST=libei
e2e_output_backend=libei
e2e_compatibility_mode=tracked-focus
e2e_input_source=${E2E_INPUT_SOURCE:-stdin}

# Activate the window whose app_id is $1 through KWin scripting.
activate() {
    script="$case_dir/activate-$1.js"
    cat >"$script" <<EOF
const target = workspace.windowList().find((window) => window.resourceClass === "$1");
if (target) {
    workspace.activeWindow = target;
}
EOF
    plugin="wx-e2e-activate-$$-$1"
    id=$(gdbus call --session --dest org.kde.KWin --object-path /Scripting \
        --method org.kde.kwin.Scripting.loadScript "$script" "$plugin" 2>>"$case_dir/kwin-script.log" |
        sed -n 's/^(\([0-9-]*\),)$/\1/p')
    if [ -z "$id" ] || [ "$id" -lt 0 ]; then
        log "could not load activation script for $1"
        return 1
    fi
    gdbus call --session --dest org.kde.KWin --object-path "/Scripting/Script$id" \
        --method org.kde.kwin.Script.run >/dev/null 2>>"$case_dir/kwin-script.log"
    gdbus call --session --dest org.kde.KWin --object-path /Scripting \
        --method org.kde.kwin.Scripting.unloadScript "$plugin" >/dev/null 2>>"$case_dir/kwin-script.log"
    wait_until 30 sink_focused "$1"
}

route_ready() {
    if [ "$e2e_input_source" = input-method ]; then
        case "$1" in
            *"source=input-method"*"backend=input-method-v2"*"state=connected"*"window_tracker_connected=true"*)
                return 0
                ;;
            *) return 1 ;;
        esac
    fi
    # Output is connected once the route reports its assurance, and focus
    # checks need the KWin tracker.
    case "$1" in
        *"window_tracker_connected=true"*) ;;
        *) return 1 ;;
    esac
    case "$1" in
        *"insertion_assurance=compatibility"*) return 0 ;;
    esac
    return 1
}

# --- runner -------------------------------------------------------------

if ! wait_until 50 gdbus introspect --session --dest org.kde.KWin --object-path /Scripting >/dev/null 2>&1; then
    printf 'unverified session KWin did not appear on the private bus\n' >>"$results"
    exit 0
fi
relay_pid=
# Whatever happens, leave no process behind: KWin ends the session when
# this script exits, but background children would outlive it.
stop_everything() {
    end_case
    if [ -n "$relay_pid" ]; then
        kill "$relay_pid" 2>/dev/null || true
        wait "$relay_pid" 2>/dev/null || true
    fi
}
trap stop_everything EXIT
trap 'exit 1' INT TERM
"$bin/wx-e2e-eis-relay" "$LIBEI_SOCKET" >>"$work/relay.log" 2>&1 &
relay_pid=$!
if ! wait_until 50 test -S "$LIBEI_SOCKET"; then
    relay_diagnostic=$(tail -5 "$work/relay.log" 2>/dev/null | tr '\n' ' ' | cut -c 1-512)
    printf 'unverified session the EIS relay did not start: %s\n' \
        "${relay_diagnostic:-no relay output}" >>"$results"
    exit 0
fi

if [ "$e2e_input_source" = input-method ]; then
    run_scenarios "input-method-expansion input-method-password-field"
else
    run_scenarios "expansion-replaces-trigger word-boundary-reinserts-delimiter cursor-placement
multiline-replacement unreachable-character-is-refused app-filter-follows-focus
command-expansion-completes focus-change-during-expansion target-closes-during-expansion config-reload daemon-restart"
fi
