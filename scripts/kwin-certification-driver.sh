#!/bin/sh
# KWin certification driver for scripts/run-certification-driver.sh.
#
#   WAYEXPAND_KWIN_CERTIFICATION_STORE=FILE kwin-certification-driver.sh SCENARIO
#
# The runner calls this once per scenario × layout × client cell, with
# WAYEXPAND_CERTIFICATION_* describing the cell. Cells are answered from a
# store written by scripts/kwin-certification-prepare.sh, which ran the
# scenarios against real clients in private KWin sessions. Exit codes follow
# the driver contract: 0 pass, 1 fail, 2 UNVERIFIED, 3 unsupported-by-design.
#
# The store only proves what the private sessions exercised: libei output
# into real clients, with keyboard capture simulated over stdin. Every cell
# that needs physical capture, hardware, or the real session lifecycle is
# UNVERIFIED here, whatever the store says, and so is any cell whose store
# does not match the build, KWin version and route under certification.
set -eu

scenario=${1:?usage: kwin-certification-driver.sh SCENARIO}
layout=${WAYEXPAND_CERTIFICATION_LAYOUT:?}
app=${WAYEXPAND_CERTIFICATION_TARGET_APP:?}
compositor=${WAYEXPAND_CERTIFICATION_COMPOSITOR:?}
backend=${WAYEXPAND_CERTIFICATION_BACKEND:?}
version=${WAYEXPAND_CERTIFICATION_VERSION:?}

unverified() {
    printf 'UNVERIFIED: %s\n' "$1"
    exit 2
}

[ "$compositor" = kde ] || unverified "this driver only exercises KWin, not $compositor"
# The private sessions use the libei output that the evdev+libei route
# shares; the IBus route is not exercised at all.
[ "$backend" = evdev+libei ] || unverified "backend $backend is not exercised by the headless KWin sessions"

case "$scenario" in
    password-field)
        printf '%s\n' 'unsupported-by-design: evdev capture has no sensitive-field signal (docs/adr/0001-evdev-best-effort-semantics.md)'
        exit 3
        ;;
    printable-press-release | backspace | escape | arrow-navigation | function-keys | \
        held-keys-repeat | held-modifiers | repeated-keys | ctrl-alt-super-chords | \
        application-shortcuts | compositor-shortcuts | media-keys | \
        held-modifier-unsupported-key | caps-lock | fast-typing | modifier-navigation | \
        dead-key-committed-text | compose-committed-text | expansion-after-committed-composition)
        unverified 'keyboard capture is simulated over stdin in the private session'
        ;;
    key-pass-through-*)
        unverified 'key pass-through needs a capturing route; the private session simulates capture'
        ;;
    keyboard-hotplug | bluetooth-keyboard-reconnect | multiple-keyboards)
        unverified 'needs physical input devices; the private session has none'
        ;;
    suspend-resume | compositor-restart)
        unverified 'not exercised by the private headless session'
        ;;
    portal-revocation)
        unverified 'the private session reaches EIS through a relay, not the RemoteDesktop portal'
        ;;
    picker-same-app-title-focus-isolation)
        unverified 'the snippet picker is not driven by the private session'
        ;;
    electron-client)
        case "$app" in
            electron-*) ;;
            *) unverified "$app is not an Electron client" ;;
        esac
        ;;
esac

store=${WAYEXPAND_KWIN_CERTIFICATION_STORE:-}
[ -n "$store" ] && [ -r "$store" ] ||
    unverified 'WAYEXPAND_KWIN_CERTIFICATION_STORE does not name a readable store; run scripts/kwin-certification-prepare.sh'
store_field() {
    sed -n "s/^$1=//p" "$store" | head -n 1
}
[ "$(store_field schema)" = 1 ] || unverified 'unsupported certification store schema'
[ "$(store_field kwin_version)" = "$version" ] ||
    unverified "store was collected on KWin $(store_field kwin_version), not $version"
[ "$(store_field source_tree_state)" = clean ] ||
    unverified 'store was collected from a modified source tree'
cli_commit=$("${WAYEXPAND_CLI:-wayexpand}" --version 2>/dev/null |
    sed -n 's/.* (commit \([^)]*\))$/\1/p') || cli_commit=
[ -n "$cli_commit" ] && [ "$cli_commit" != unknown ] ||
    unverified 'could not read the build identity of the CLI under certification'
[ "$(store_field commit)" = "$cli_commit" ] ||
    unverified "store tested commit $(store_field commit), not $cli_commit"

cell=$(grep -F -m 1 "cell $scenario|$layout|$app " "$store") ||
    unverified "no result for $scenario on $layout in $app (client or scenario not driven)"
# "cell SCENARIO|LAYOUT|APP OUTCOME DETAIL..."
outcome=$(printf '%s\n' "$cell" | cut -d ' ' -f 3)
detail=$(printf '%s\n' "$cell" | cut -d ' ' -f 4-)
printf 'store: %s: %s (commit %s, capture %s, output %s)\n' "$outcome" "$detail" \
    "$(store_field commit)" "$(store_field capture)" "$(store_field output_backend)"
case "$scenario" in
    focus-change-during-expansion | target-closes-during-expansion | expansion-cancellation)
        # Evdev capture hands results to its serialized output pipeline,
        # with key-release and input-quiet gating that stdin capture never
        # reaches. The observation above is kept for review, but it is not
        # evidence for how the evdev route handles keys typed meanwhile.
        unverified 'pending-output handling runs in the evdev pipeline, which stdin capture does not exercise'
        ;;
esac
case "$outcome" in
    pass) exit 0 ;;
    fail) exit 1 ;;
    unsupported-by-design) exit 3 ;;
    *) exit 2 ;;
esac
