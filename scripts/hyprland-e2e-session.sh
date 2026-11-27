#!/bin/sh
# Session driver for the nested Hyprland end-to-end harness.
#
# Runs against the private nested Hyprland that scripts/test-hyprland-e2e.sh
# starts; never run it in a real desktop session. The scenarios live in
# scripts/e2e-lib.sh. Here the daemon uses:
#   - capture: --source=stdin (the harness feeds what the "user" typed;
#     real keyboard capture needs /dev/input and is not exercised here), or
#     with E2E_INPUT_SOURCE=input-method the production input-method-v2
#     source, whose unsupported keys pass through its own virtual keyboard,
#   - output: the production wlroots virtual-keyboard backend (or the
#     input-method protocol itself in input-method mode),
#   - focus: the production Hyprland IPC window tracker on this session's
#     HYPRLAND_INSTANCE_SIGNATURE, with compatibility_mode = "tracked-focus".
# The "user" types into the focused sink with wx-e2e-type through its own
# wlroots virtual keyboard, separate from the daemon's.
set -u

# shellcheck source=scripts/e2e-lib.sh
. "$(dirname -- "$0")/e2e-lib.sh"

export WX_E2E_TYPIST=wlroots
e2e_output_backend=wlroots
e2e_compatibility_mode=tracked-focus
# This private harness deliberately exercises explicitly enabled paced
# compatibility output; production defaults remain limited to 8 characters.
e2e_keystroke_max_chars=250
e2e_input_source=${E2E_INPUT_SOURCE:-stdin}
# Hyprland activates an input method only when a field enables text input,
# so an input method that connects after the field was focused stays
# inactive. Start the daemon first, as at login.
# shellcheck disable=SC2034 # read by scripts/e2e-lib.sh
e2e_input_method_starts_first=1

# Focus the window whose class (app_id) is $1 through Hyprland IPC.
activate() {
    hyprctl dispatch focuswindow "class:^($1)\$" >>"$case_dir/hyprctl.log" 2>&1 || return 1
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
    # Focus checks need the Hyprland tracker.
    case "$1" in
        *"window_tracker_connected=true"*) ;;
        *) return 1 ;;
    esac
    if [ "${e2e_expect_blocked_route:-0}" = 1 ]; then
        case "$1" in
            *"source=stdin"*"backend=wlroots-virtual-keyboard"*"insertion_assurance=blocked"*)
                return 0
                ;;
            *) return 1 ;;
        esac
    fi
    case "$1" in
        *"backend=wlroots-virtual-keyboard"*) ;;
        *) return 1 ;;
    esac
    case "$1" in
        *"insertion_assurance=compatibility"*) return 0 ;;
    esac
    return 1
}

# --- runner -------------------------------------------------------------

if ! wait_until 50 hyprctl version >/dev/null 2>&1; then
    printf 'unverified session Hyprland did not answer on its IPC socket\n' >>"$results"
    exit 0
fi
trap end_case EXIT
trap 'exit 1' INT TERM

if [ "$e2e_input_source" = input-method ]; then
    run_scenarios "input-method-expansion input-method-pass-through input-method-password-field"
    exit 0
fi

run_scenarios "expansion-replaces-trigger word-boundary-reinserts-delimiter cursor-placement
multiline-replacement unicode-insertion app-filter-follows-focus app-filter-fails-closed
command-expansion-completes focus-change-during-expansion target-closes-during-expansion
config-reload daemon-restart clipboard-variable
clipboard-empty-fails-closed keystroke-route-refuses-without-consent"
