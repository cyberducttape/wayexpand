#!/bin/sh
# Guided live certification of the input-method-v2 route in a real Sway or
# Hyprland session.
#
#   scripts/wlroots-live-certification.sh --reviewed-by NAME --physical-keyboard MODEL
#       [--layout PROFILE] [--output-dir DIR]
#
# Run it from a clean checkout of the exact commit being certified, in your
# own Sway or Hyprland session, with that build installed and the daemon
# running on the input-method-v2 route (`wayexpand setup` selects it). It
# checks that the running daemon and CLI are that build and that the
# compositor's window tracker is connected, adds a few clearly marked test
# snippets to your library (your own snippets are not touched, and the file is
# restored when the session ends, even on Ctrl-C), then walks you through each
# cell a person can perform: a real GTK 3 or Qt 6 window opens, you type what
# it asks on the physical keyboard, and the outcome comes from the text the
# toolkit received. Everything else in the matrix is recorded UNVERIFIED with
# its reason. The result is an evidence file from
# scripts/certify-compositor.sh; review it before submitting it.
set -eu
umask 077

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
reviewed_by=
physical_keyboard=
live_layout=us
output_dir=
cli=${WAYEXPAND_CLI:-wayexpand}

usage() {
    printf '%s\n' "usage: $0 --reviewed-by NAME --physical-keyboard MODEL [--layout PROFILE] [--output-dir DIR]"
}
while [ "$#" -gt 0 ]; do
    case "$1" in
        --reviewed-by) reviewed_by=${2:?missing value for --reviewed-by}; shift 2 ;;
        --physical-keyboard) physical_keyboard=${2:?missing value for --physical-keyboard}; shift 2 ;;
        --layout) live_layout=${2:?missing value for --layout}; shift 2 ;;
        --output-dir) output_dir=${2:?missing value for --output-dir}; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done
[ -n "$reviewed_by" ] && [ -n "$physical_keyboard" ] || { usage >&2; exit 2; }

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

# ---- Preflight: the real session, the exact build, a working route ----
for tool in jq python3 timeout sha256sum git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done
[ -n "${WAYLAND_DISPLAY:-}" ] ||
    fail 'run this in your Sway or Hyprland session, not over SSH or in a virtual session'
if [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
    compositor=hyprland
    command -v hyprctl >/dev/null 2>&1 || fail 'hyprctl is required to read the Hyprland version'
    compositor_version=$(hyprctl version -j 2>/dev/null | jq -r '.tag // .version // empty' | sed 's/^v//; s/-.*//')
elif [ -n "${SWAYSOCK:-}" ]; then
    compositor=sway
    compositor_version=$(sway --version 2>/dev/null | awk '{print $3}')
else
    fail 'neither HYPRLAND_INSTANCE_SIGNATURE nor SWAYSOCK is set; run this in a Sway or Hyprland session'
fi
[ -n "$compositor_version" ] || fail "could not read the $compositor version"
[ -z "$(git -C "$project_dir" status --porcelain)" ] ||
    fail 'the checkout has uncommitted changes; certify a clean commit'
commit=$(git -C "$project_dir" rev-parse --short=12 HEAD)
cli_commit=$("$cli" --version 2>/dev/null | sed -n 's/.* (commit \([^)]*\))$/\1/p')
[ "$cli_commit" = "$commit" ] ||
    fail "the installed CLI is commit '${cli_commit:-unknown}', not $commit; install this build first"
status_json=$("$cli" status --json 2>/dev/null) ||
    fail "the WayExpand daemon is not running; start it and check 'wayexpand status'"
printf '%s' "$status_json" | jq -e --arg commit "$commit" '.daemon_commit == $commit' >/dev/null ||
    fail "the running daemon is not commit $commit; restart it with this build (systemctl --user restart wayexpand)"
printf '%s' "$status_json" | jq -e '.source == "input-method" and .backend == "input-method-v2"' >/dev/null ||
    fail "the daemon is not on the input-method-v2 route ($(printf '%s' "$status_json" | jq -r '"\(.source) + \(.backend)"')); run 'wayexpand setup'"
printf '%s' "$status_json" | jq -e '.window_tracker_connected == true' >/dev/null ||
    fail "the $compositor window tracker is not connected; see 'wayexpand debug' (WX-WINDOW-ENV-001)"
python3 -c 'import gi; gi.require_version("Gtk", "3.0"); from gi.repository import Gtk' 2>/dev/null ||
    fail 'python3-gi with GTK 3 is required for the GTK readback client'
python3 -c 'import PyQt6.QtWidgets' 2>/dev/null ||
    fail 'python3-pyqt6 is required for the Qt readback client'

if [ -z "$output_dir" ]; then
    output_dir="$project_dir/target/live-certification-$compositor-$commit-$(date -u +%Y%m%dT%H%M%SZ)"
fi
if [ -L "$output_dir" ]; then
    fail 'the output directory must not be a symlink'
elif [ -e "$output_dir" ]; then
    [ -d "$output_dir" ] || fail 'the output path is not a directory'
else
    mkdir -p "$output_dir"
fi
config=$(printf '%s' "$status_json" | jq -r '.config_active_path // empty')
[ -n "$config" ] && [ -f "$config" ] || fail "the daemon reports no readable configuration (${config:-none})"

# Reusing an evidence directory with recovery state from an interrupted run
# could make the EXIT trap restore a backup created by a different session.
# Keep unrelated evidence files, but never arm cleanup over stale recovery
# metadata or an unfinished config replacement.
for recovery_file in \
    "$output_dir/original-configuration.toml" \
    "$output_dir/original-configuration.mode" \
    "$output_dir/modified-configuration.sha256" \
    "$output_dir/modified-configuration.sha256.tmp" \
    "$config.live-certification.tmp"; do
    [ ! -e "$recovery_file" ] && [ ! -L "$recovery_file" ] ||
        fail "recovery state already exists at $recovery_file; inspect it before reusing this output directory"
done

# ---- Test snippets: appended in a marked block, removed on exit ----
"$cli" backup >/dev/null || fail 'could not back up the configuration first'
generation_before=$(printf '%s' "$status_json" | jq -r '.config_generation')
# Keep in sync with scripts/kde-live-certification.sh and the driver.
snippets=$(cat <<'EOF'
# BEGIN wayexpand-live-certification (temporary; removed when the session ends)
[[expansion]]
id = "6f0a3b1e-0c11-4d6a-9f00-00000000c0a1"
trigger = ";wxa"
replacement = "alpha"

[[expansion]]
id = "6f0a3b1e-0c11-4d6a-9f00-00000000c0a2"
trigger = ";wxu"
replacement = "café ✓ naïve 日本語"

[[expansion]]
id = "6f0a3b1e-0c11-4d6a-9f00-00000000c0a3"
trigger = ";wxl"
replacement = "WayExpand live certification: the quick brown fox jumps over the lazy dog, then the lazy dog naps while the quick brown fox counts to twenty. WayExpand live certification: the quick brown fox jumps over the lazy dog, then the lazy dog naps while the quick brown fox counts to twenty. "
# END wayexpand-live-certification
EOF
)
edit_config() {
    python3 "$project_dir/scripts/live-certification-config.py" "$1" "$config" "$output_dir" "$snippets"
}
trap 'edit_config restore' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
edit_config add || fail 'could not add the test snippets'
"$cli" config validate "$config" >/dev/null 2>&1 ||
    fail 'the configuration is invalid with the test snippets (a trigger may collide with ;wxa, ;wxu or ;wxl)'
deadline=$(( $(date +%s) + 20 ))
until [ "$("$cli" status --json 2>/dev/null | jq -r '.config_generation')" != "$generation_before" ]; do
    [ "$(date +%s)" -lt "$deadline" ] || fail 'the daemon did not reload the configuration within 20 seconds'
    sleep 0.5
done

cat <<EOF

WayExpand live certification — commit $commit, $compositor $compositor_version, layout $live_layout
Keyboard: $physical_keyboard   Reviewer: $reviewed_by

A test window opens for each cell. Click into it, type exactly what is shown
on the physical keyboard, then wait; it closes by itself. Type only the test
text. If you make a typing mistake the cell fails: finish, then rerun.
The temporary snippets are ;wxa, ;wxu and ;wxl.

EOF
printf '%s' 'Press Enter to begin (Ctrl-C to cancel)… '
read -r _

layouts=$(jq -r '.required_layout_profiles | join(",")' "$project_dir/tests/certification/compositor-matrix.json")
apps=gtk3-textview,qt6-plaintextedit,password-field,terminal-foot,browser-firefox,electron-vscode,gtk4-editor-gnome-text-editor
mkdir -p "$output_dir/state"
results="$output_dir/results.txt"
WAYEXPAND_LIVE_LAYOUT=$live_layout WAYEXPAND_LIVE_STATE_DIR="$output_dir/state" \
    sh "$project_dir/scripts/run-certification-driver.sh" \
    --driver "$project_dir/scripts/live-certification-driver.sh" \
    --compositor "$compositor" --version "$compositor_version" --backend input-method-v2 \
    --layout "$layouts" --target-apps "$apps" \
    --timeout 200 --deadline 7200 \
    --output "$results" --log-dir "$output_dir/logs" || true

printf '\n%s ' "Did you type every prompt on the physical keyboard ($physical_keyboard)? [y/N]"
read -r answer
physical_flag=
case "$answer" in
    y | Y | yes) physical_flag=--physical-input-confirmed ;;
esac

gtk_version=$(python3 -c 'import gi; gi.require_version("Gtk", "3.0"); from gi.repository import Gtk; print("%d.%d.%d" % (Gtk.get_major_version(), Gtk.get_minor_version(), Gtk.get_micro_version()))')
qt_version=$(python3 -c 'from PyQt6.QtCore import QT_VERSION_STR; print(QT_VERSION_STR)')
client_versions=$(jq -nc --arg gtk "$gtk_version" --arg qt "$qt_version" '
    {"application_version": "not-exercised-by-live-driver", "toolkit_name": "none", "toolkit_version": "none"} as $none |
    {
      "gtk3-textview": {"application_version": "certification gtk_client.py", "toolkit_name": "GTK", "toolkit_version": $gtk},
      "password-field": {"application_version": "certification gtk_client.py --password", "toolkit_name": "GTK", "toolkit_version": $gtk},
      "qt6-plaintextedit": {"application_version": "certification qt_client.py", "toolkit_name": "Qt", "toolkit_version": $qt},
      "terminal-foot": $none, "browser-firefox": $none, "electron-vscode": $none, "gtk4-editor-gnome-text-editor": $none
    }')
evidence="$output_dir/$compositor-live-input-method-v2-$commit.json"
# shellcheck disable=SC2086 # physical_flag is empty or one flag.
sh "$project_dir/scripts/certify-compositor.sh" \
    --compositor "$compositor" --version "$compositor_version" --backend input-method-v2 \
    --layout "$layouts" --target-apps "$apps" --client-versions "$client_versions" \
    --physical-keyboard "$physical_keyboard" --physical-evidence-ref "live-certification-$compositor-$commit" \
    $physical_flag --reviewed-by "$reviewed_by" \
    --cli "$cli" --format json --output "$evidence" --results "$results" || true

printf '\n%s\n' "Results: $(cut -d= -f2 "$results" | sort | uniq -c | tr '\n' ' ')"
printf '%s\n' "Evidence: $evidence"
printf '%s\n' "Driver logs: $output_dir/logs"
printf '%s\n' 'Review the evidence, then copy it into docs/evidence/ in a pull request.'
