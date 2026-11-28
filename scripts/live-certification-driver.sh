#!/bin/sh
# Certification driver for a live, physical desktop session: KDE Plasma on
# the IBus route, or Sway or Hyprland on the input-method-v2 route. Started
# once per matrix cell by scripts/run-certification-driver.sh, which
# scripts/kde-live-certification.sh or scripts/wlroots-live-certification.sh
# runs after its preflight; do not run it directly.
#
# For the cells a person can perform, it opens a real GTK 3 or Qt 6 readback
# client (scripts/certification-clients), tells the person at the physical
# keyboard what to type, and decides the outcome from the text the toolkit
# actually received, never from the person's judgment. Cells that need
# another layout, another client, hardware events, or timing no person can
# hit are reported UNVERIFIED with the reason.
#
# Exit status (driver contract): 0 pass, 1 fail, 2 unverified,
# 3 unsupported-by-design.
#
# The runner starts each cell under timeout(1), outside the terminal's
# foreground process group, so this driver writes prompts to /dev/tty but
# never reads from it; completion is detected from the client's state file.
set -eu

scenario=${1:?scenario is required}
app=${WAYEXPAND_CERTIFICATION_TARGET_APP:?}
layout=${WAYEXPAND_CERTIFICATION_LAYOUT:?}
backend=${WAYEXPAND_CERTIFICATION_BACKEND:?}
live_layout=${WAYEXPAND_LIVE_LAYOUT:-}
state_dir=${WAYEXPAND_LIVE_STATE_DIR:-}
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# Tests substitute scripted clients; a live session uses the real ones.
clients=${WAYEXPAND_LIVE_CLIENTS:-"$project_dir/scripts/certification-clients"}

unverified() {
    printf 'unverified: %s\n' "$*"
    exit 2
}

# The test snippets the live-certification wrappers add for the session.
# Keep these in sync with the snippet block in both wrappers.
long_text='WayExpand live certification: the quick brown fox jumps over the lazy dog, then the lazy dog naps while the quick brown fox counts to twenty. '
long_text="$long_text$long_text"
unicode_text='café ✓ naïve 日本語'

case "$backend" in
    ibus | input-method-v2) ;;
    *) unverified "the live driver exercises only the IBus and input-method-v2 routes, not $backend" ;;
esac
[ -n "$live_layout" ] && [ -n "$state_dir" ] ||
    unverified 'run through scripts/kde-live-certification.sh or scripts/wlroots-live-certification.sh, which set up the live session'
[ "$layout" = "$live_layout" ] ||
    unverified "the session typed on layout $live_layout; switch the desktop to $layout and rerun to cover it"

case "$app" in
    gtk3-textview) client=gtk ;;
    qt6-plaintextedit) client=qt ;;
    password-field) client=password ;;
    *) unverified "the live driver has no readback client for $app" ;;
esac

# keys: what the person types. expected: the exact text the client must hold.
case "$scenario" in
    printable-press-release)
        keys=';wxa'
        if [ "$client" = password ]; then
            # Password-purpose fields are sensitive on both routes; nothing
            # may expand.
            expected=';wxa'
        else
            expected='alpha'
        fi
        ;;
    backspace)
        keys=';wxq, then Backspace once, then a'
        expected='alpha'
        ;;
    fast-typing)
        keys='go ;wxa now   (as fast as you can, in one burst)'
        expected='go alpha now'
        ;;
    unicode-combining)
        keys=';wxu'
        expected=$unicode_text
        ;;
    second-trigger-during-nonatomic-replacement)
        keys=';wxl;wxa   (in one fast burst, without pausing after ;wxl)'
        expected="${long_text}alpha"
        ;;
    modifier-during-nonatomic-replacement)
        keys=';wxl, then immediately hold Shift and type z'
        expected="${long_text}Z"
        ;;
    focus-change-during-expansion | target-closes-during-expansion | expansion-cancellation)
        unverified 'the replacement is committed in a single protocol call; a person cannot time a focus change or close inside it'
        ;;
    *)
        unverified "$scenario is not a live-typed scenario in this driver"
        ;;
esac
if [ "$client" = password ] && [ "$scenario" != printable-press-release ]; then
    unverified 'the password field is checked only for sensitive-field refusal'
fi

say() {
    # Without a controlling terminal (tests, CI) prompts go to the log.
    { printf '%s\n' "$*" >/dev/tty; } 2>/dev/null || printf '%s\n' "$*" >&2
}

state="$state_dir/$scenario-$app.json"
rm -f -- "$state"
# The clients use this as both the app ID and the window title.
title="wayexpand-live-certification-$app"
case "$client" in
    gtk) python3 "$clients/gtk_client.py" "$title" "$state" >/dev/null 2>&1 & ;;
    password) python3 "$clients/gtk_client.py" "$title" "$state" --password >/dev/null 2>&1 & ;;
    qt) python3 "$clients/qt_client.py" "$title" "$state" >/dev/null 2>&1 & ;;
esac
client_pid=$!
trap 'kill "$client_pid" 2>/dev/null || true' EXIT INT TERM

field() {
    jq -r "$1" "$state" 2>/dev/null || true
}
now_ms() {
    date +%s%3N
}

say ''
say "── $scenario · $app"
say "1. Click into the window \"$title\"."
deadline=$(( $(now_ms) + 60000 ))
until [ "$(field .focused)" = true ]; do
    kill -0 "$client_pid" 2>/dev/null || unverified "the $client readback client exited before it was focused"
    [ "$(now_ms)" -lt "$deadline" ] || unverified 'the test window was not focused within 60 seconds'
    sleep 0.2
done
say "2. Type exactly:  $keys"
say '   Then keep your hands off the keyboard; the window closes by itself.'

# The outcome is the text once it has been stable for 3 seconds, so late
# duplicate output still counts against the cell.
last=''
changed=$(now_ms)
deadline=$(( $(now_ms) + 90000 ))
while :; do
    text=$(field .text)
    if [ "$text" != "$last" ]; then
        last=$text
        changed=$(now_ms)
    fi
    if [ -n "$text" ] && [ $(( $(now_ms) - changed )) -ge 3000 ]; then
        break
    fi
    if [ "$(now_ms)" -ge "$deadline" ]; then
        [ -n "$text" ] || unverified 'nothing was typed within 90 seconds'
        break
    fi
    sleep 0.2
done

if [ "$text" = "$expected" ]; then
    say '   ✓ received the expected text'
    printf 'pass: %s received exactly the expected %s characters\n' "$app" "${#expected}"
    exit 0
fi
say '   ✗ the text does not match (rerun the session if this was a typing mistake)'
# Test text only; no user content reaches these clients.
printf 'fail: expected %s characters, received %s: %s\n' "${#expected}" "${#text}" \
    "$(printf '%s' "$text" | jq -Rsc .)"
exit 1
