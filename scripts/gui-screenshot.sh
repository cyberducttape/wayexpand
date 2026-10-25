#!/usr/bin/env bash
# Render the WayExpand GUI off-screen and save a PNG, for checking its look.
#
#   scripts/gui-screenshot.sh CONFIG OUTPUT.png [WIDTHxHEIGHT]
#
# Runs a debug wayexpand-gui inside a private, headless KWin
# (kwin_wayland --virtual) with its own D-Bus session, runtime directory and
# configuration directory, so nothing touches the running desktop, its
# daemon, or the user's settings.
#
#   WAYEXPAND_GUI_SCREENSHOT_VIEW  screen to open first (see screenshot.rs)
#   WAYEXPAND_GUI_SCREENSHOT_PACK  color pack code, for example retro_80s_neon
#   WAYEXPAND_GUI_SCREENSHOT_DAEMON=1  also run a sandboxed daemon that reads
#                                  stdin and types nothing, so the header
#                                  shows a running service
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
source "$project_dir/scripts/private-home-cleanup.sh"

for tool in kwin_wayland dbus-run-session convert; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 1
    }
done

cargo build --quiet --locked -p wayexpand-gui -p wayexpand-daemon \
    --manifest-path "$project_dir/Cargo.toml"
gui="$project_dir/target/debug/wayexpand-gui"
daemon="$project_dir/target/debug/wayexpand-daemon"

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-gui-shot.XXXXXXXX")
# The compositor's socket path must fit in 108 bytes, so its runtime
# directory gets a short path of its own.
runtime=$(mktemp -d /tmp/wxshot.XXXXXX)
cleanup() {
    # KWin's session wrapper backgrounds the optional daemon pipeline. Reap
    # every process with this run's private HOME before deleting its runtime.
    reap_private_home_processes "$work/home"
    sleep 0.2
    reap_private_home_processes "$work/home"
    rm -rf "$work" "$runtime"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
chmod 700 "$runtime"
# The GUI opens its default configuration path, which must be private. It
# sits under HOME so the status bar shows ~/.config/wayexpand/expansions.toml.
config_home="$work/home/.config"
mkdir -m 700 "$work/home" "$config_home" "$config_home/wayexpand"
cp -- "$config" "$config_home/wayexpand/expansions.toml"
chmod 600 "$config_home/wayexpand/expansions.toml"
if [ -n "${WAYEXPAND_GUI_SCREENSHOT_PACK:-}" ]; then
    printf 'language = "en"\ncolorpack = "%s"\n' "$WAYEXPAND_GUI_SCREENSHOT_PACK" \
        >"$config_home/wayexpand/gui-prefs.toml"
    chmod 600 "$config_home/wayexpand/gui-prefs.toml"
fi

# kwin_wayland --exit-with-session takes one program; this wrapper starts
# the optional daemon and then the GUI.
session="$work/session.sh"
cat >"$session" <<EOF
#!/bin/sh
if [ "\${WAYEXPAND_GUI_SCREENSHOT_DAEMON:-}" = 1 ]; then
    sleep 120 | '$daemon' --source=stdin --backend=none >'$work/daemon.log' 2>&1 &
    sleep 2
fi
exec '$gui'
EOF
chmod 700 "$session"

env -i \
    PATH="$PATH" \
    HOME="$work/home" \
    LANG="${LANG:-C.UTF-8}" \
    XDG_RUNTIME_DIR="$runtime" \
    XDG_CONFIG_HOME="$config_home" \
    WAYEXPAND_SOCKET="$runtime/wayexpand.sock" \
    WAYEXPAND_GUI_SCREENSHOT="$work/shot.ppm" \
    WAYEXPAND_GUI_SCREENSHOT_VIEW="${WAYEXPAND_GUI_SCREENSHOT_VIEW:-}" \
    WAYEXPAND_GUI_SCREENSHOT_DAEMON="${WAYEXPAND_GUI_SCREENSHOT_DAEMON:-}" \
    QT_QPA_PLATFORM=offscreen \
    timeout 90 dbus-run-session -- \
    kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
    --width "$width" --height "$height" --socket wayexpand-shot \
    --exit-with-session "$session" \
    >"$work/kwin.log" 2>&1 || true

if [ ! -s "$work/shot.ppm" ]; then
    printf '%s\n' 'error: no screenshot was taken; compositor log:' >&2
    tail -20 "$work/kwin.log" >&2
    exit 1
fi
convert "$work/shot.ppm" "$output"
printf 'Wrote %s\n' "$output"
