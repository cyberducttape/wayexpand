#!/bin/sh
# Session driver for the headless Sway end-to-end harness.
#
# Runs against the private headless Sway that scripts/test-sway-e2e.sh
# starts; never run it in a real desktop session. The scenarios live in
# scripts/e2e-lib.sh. Here the daemon uses:
#   - capture: --source=stdin (the harness feeds what the "user" typed;
#     real keyboard capture needs /dev/input and is not exercised here),
#   - output: the production wlroots virtual-keyboard backend,
#   - focus: none -- WayExpand has no Sway window tracker, so the route
#     runs with compatibility_mode = "untracked-focus" and app_filter
#     snippets must fail closed.
# The "user" types into the focused sink with wx-e2e-type through its own
# wlroots virtual keyboard, separate from the daemon's.
#
# Input-method-v2 capture is not exercised: the daemon only starts it with
# libei key pass-through, and Sway offers no libei/EIS server.
set -u

# shellcheck source=scripts/e2e-lib.sh
. "$(dirname -- "$0")/e2e-lib.sh"

export WX_E2E_TYPIST=wlroots
e2e_output_backend=wlroots
e2e_compatibility_mode=untracked-focus

# Focus the window whose app_id is $1 through Sway IPC.
activate() {
    swaymsg "[app_id=\"$1\"] focus" >>"$case_dir/swaymsg.log" 2>&1 || return 1
    wait_until 30 sink_focused "$1"
}

route_ready() {
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

if ! wait_until 50 swaymsg -t get_version >/dev/null 2>&1; then
    printf 'unverified session Sway did not answer on its IPC socket\n' >>"$results"
    exit 0
fi
trap end_case EXIT
trap 'exit 1' INT TERM

run_scenarios "expansion-replaces-trigger word-boundary-reinserts-delimiter cursor-placement
multiline-replacement unreachable-character-is-refused app-filter-fails-closed
command-expansion-completes config-reload daemon-restart clipboard-variable
clipboard-empty-fails-closed keystroke-route-refuses-without-consent"
