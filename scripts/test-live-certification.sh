#!/bin/sh
# Check the live certification driver and its configuration editor
# without a desktop: scripted stand-ins for the readback clients write the
# state a real client would, so the outcome rules, unverified reasons and
# configuration restore are tested exactly as the live session uses them.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
driver="$project_dir/scripts/live-certification-driver.sh"
editor="$project_dir/scripts/live-certification-config.py"
test_dir=$(mktemp -d)
trap 'rm -rf -- "$test_dir"' EXIT INT TERM
fake="$test_dir/clients"
mkdir -p "$fake" "$test_dir/state"

# A stand-in client: focus at once, then deliver $FAKE_TEXT as the toolkit
# would. Argument order matches gtk_client.py and qt_client.py.
cat >"$fake/gtk_client.py" <<'PY'
import json, os, sys, time
state = sys.argv[2]
def write(text, focused=True):
    tmp = state + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump({"app_id": sys.argv[1], "focused": focused, "text": text, "cursor": len(text)}, handle)
    os.replace(tmp, state)
write("", False)
time.sleep(0.3)
write("")
time.sleep(0.3)
write(os.environ.get("FAKE_TEXT", ""))
time.sleep(30)
PY
cp "$fake/gtk_client.py" "$fake/qt_client.py"

run_cell() {
    # run_cell EXPECTED_EXIT SCENARIO APP [BACKEND] [LAYOUT]
    expected=$1 scenario=$2 app=$3 backend=${4:-ibus} layout=${5:-us}
    export FAKE_TEXT="${FAKE_TEXT:-}"
    set +e
    output=$(WAYEXPAND_CERTIFICATION_TARGET_APP=$app WAYEXPAND_CERTIFICATION_LAYOUT=$layout \
        WAYEXPAND_CERTIFICATION_BACKEND=$backend WAYEXPAND_LIVE_LAYOUT=us \
        WAYEXPAND_LIVE_STATE_DIR="$test_dir/state" WAYEXPAND_LIVE_CLIENTS="$fake" \
        sh "$driver" "$scenario" 2>/dev/null </dev/null)
    status=$?
    set -e
    if [ "$status" -ne "$expected" ]; then
        printf 'error: %s/%s/%s/%s exited %s, expected %s: %s\n' \
            "$scenario" "$app" "$backend" "$layout" "$status" "$expected" "$output" >&2
        exit 1
    fi
}

# Cells nobody can perform here are unverified, without opening a window.
run_cell 2 printable-press-release gtk3-textview evdev+libei
run_cell 2 printable-press-release gtk3-textview ibus de
run_cell 2 printable-press-release browser-firefox
run_cell 2 keyboard-hotplug gtk3-textview
run_cell 2 focus-change-during-expansion gtk3-textview
run_cell 2 backspace password-field
run_cell 2 printable-press-release gtk3-textview evdev+wlroots

# The outcome is the exact text the client received.
FAKE_TEXT=alpha run_cell 0 printable-press-release gtk3-textview
FAKE_TEXT=alpha run_cell 0 backspace qt6-plaintextedit
FAKE_TEXT='go alpha now' run_cell 0 fast-typing gtk3-textview
FAKE_TEXT='café ✓ naïve 日本語' run_cell 0 unicode-combining qt6-plaintextedit
FAKE_TEXT=';wxa' run_cell 0 printable-press-release password-field
FAKE_TEXT=alpha run_cell 1 printable-press-release password-field
FAKE_TEXT=alphaalpha run_cell 1 printable-press-release gtk3-textview
FAKE_TEXT=';wxa' run_cell 1 printable-press-release gtk3-textview
long='WayExpand live certification: the quick brown fox jumps over the lazy dog, then the lazy dog naps while the quick brown fox counts to twenty. '
FAKE_TEXT="$long${long}alpha" run_cell 0 second-trigger-during-nonatomic-replacement gtk3-textview
FAKE_TEXT="$long${long}lpha" run_cell 1 second-trigger-during-nonatomic-replacement gtk3-textview
FAKE_TEXT="$long${long}Z" run_cell 0 modifier-during-nonatomic-replacement qt6-plaintextedit

# The input-method-v2 route (Sway, Hyprland) uses the same outcome rules.
FAKE_TEXT=alpha run_cell 0 printable-press-release gtk3-textview input-method-v2
FAKE_TEXT=';wxa' run_cell 0 printable-press-release password-field input-method-v2
FAKE_TEXT=alpha run_cell 1 printable-press-release password-field input-method-v2
run_cell 2 focus-change-during-expansion qt6-plaintextedit input-method-v2

# The configuration editor restores exact bytes, or removes only its block.
config="$test_dir/expansions.toml"
block='# BEGIN wayexpand-live-certification (temporary)
[[expansion]]
trigger = ";wxa"
replacement = "alpha"
# END wayexpand-live-certification'
original='# my comment
[[expansion]]
trigger   = ":mine"
replacement = "kept"
'
printf '%s' "$original" >"$config"
chmod 0600 "$config"
out="$test_dir/run1"
mkdir -m 0700 "$out"
python3 "$editor" add "$config" "$out" "$block"
grep -F ';wxa' "$config" >/dev/null
[ "$(stat -c %a "$config")" = 600 ]
python3 "$editor" restore "$config" "$out" "$block" >/dev/null
[ "$(cat "$config")" = "$(printf '%s' "$original")" ] || { echo 'error: restore was not exact' >&2; exit 1; }

out="$test_dir/run2"
mkdir -m 0700 "$out"
python3 "$editor" add "$config" "$out" "$block"
printf '%s\n' '[[expansion]]' 'trigger = ":during"' 'replacement = "edited meanwhile"' >>"$config"
python3 "$editor" restore "$config" "$out" "$block" >/dev/null
if grep -F ';wxa' "$config" >/dev/null; then echo 'error: test block left behind' >&2; exit 1; fi
grep -F ':during' "$config" >/dev/null || { echo 'error: concurrent edit lost' >&2; exit 1; }
grep -F ':mine' "$config" >/dev/null || { echo 'error: user snippet lost' >&2; exit 1; }

printf '%s\n' 'live certification driver test passed'
