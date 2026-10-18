#!/usr/bin/env bash
# Render the WayExpand GUI off-screen and save a PNG, for checking its look.
#
#   scripts/gui-screenshot.sh CONFIG OUTPUT.png [WIDTHxHEIGHT]
#
# Runs a debug wayexpand-gui inside a private, headless KWin
# (kwin_wayland --virtual) with its own D-Bus session, runtime directory and
# configuration directory, so nothing touches the running desktop, its
# daemon, or the user's settings. Extra environment (for example
# WAYEXPAND_GUI_SCREENSHOT_VIEW) is passed through.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    printf '%s\n' 'usage: gui-screenshot.sh CONFIG OUTPUT.png [WIDTHxHEIGHT]' >&2
    exit 2
fi
config=$(realpath -- "$1")
output=$(realpath -m -- "$2")
size=${3:-1240x850}
width=${size%x*}
height=${size#*x}
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

for tool in kwin_wayland dbus-run-session convert; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 1
    }
done

cargo build --quiet --locked -p wayexpand-gui --manifest-path "$project_dir/Cargo.toml"
gui="$project_dir/target/debug/wayexpand-gui"

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-gui-shot.XXXXXXXX")
# The compositor's socket path must fit in 108 bytes, so its runtime
# directory gets a short path of its own.
runtime=$(mktemp -d /tmp/wxshot.XXXXXX)
trap 'rm -rf "$work" "$runtime"' EXIT
chmod 700 "$runtime"
mkdir -m 700 "$work/config" "$work/home"
# The GUI opens its default configuration path, which must be private.
mkdir -m 700 "$work/config/wayexpand"
cp -- "$config" "$work/config/wayexpand/expansions.toml"
chmod 600 "$work/config/wayexpand/expansions.toml"

env -i \
    PATH="$PATH" \
    HOME="$work/home" \
    LANG="${LANG:-C.UTF-8}" \
    XDG_RUNTIME_DIR="$runtime" \
    XDG_CONFIG_HOME="$work/config" \
    WAYEXPAND_SOCKET="$runtime/no-daemon.sock" \
    WAYEXPAND_GUI_SCREENSHOT="$work/shot.ppm" \
    WAYEXPAND_GUI_SCREENSHOT_VIEW="${WAYEXPAND_GUI_SCREENSHOT_VIEW:-}" \
    QT_QPA_PLATFORM=offscreen \
    timeout 90 dbus-run-session -- \
    kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --width "$width" --height "$height" --socket wayexpand-shot \
    --exit-with-session "$gui" \
    >"$work/kwin.log" 2>&1 || true

if [ ! -s "$work/shot.ppm" ]; then
    printf '%s\n' 'error: no screenshot was taken; compositor log:' >&2
    tail -20 "$work/kwin.log" >&2
    exit 1
fi
convert "$work/shot.ppm" "$output"
printf 'Wrote %s\n' "$output"
