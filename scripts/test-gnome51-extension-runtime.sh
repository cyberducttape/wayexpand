#!/bin/sh
# Load the production WayExpand Shell extension inside a real GNOME Shell 51
# process and verify its live metadata and exported protocol. This is runtime
# compatibility evidence, not desktop keyboard or insertion certification.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
uuid=wayexpand-window-tracker@cyberducttape.github.io
extension_dir="$project_dir/gnome-extension/$uuid"
shell_version=$(gnome-shell --version 2>&1)
case "$shell_version" in
    *" 51."*|*" 51"*) ;;
    *) printf 'expected GNOME Shell 51, found: %s\n' "$shell_version" >&2; exit 1 ;;
esac

if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' 'run this compatibility test as an unprivileged user' >&2
    exit 2
fi
for tool in dbus-run-session gsettings gdbus gnome-extensions python3; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'required GNOME test tool is missing: %s\n' "$tool" >&2
        exit 2
    }
done
tracker_bin=${WX_GNOME_TRACKER_BIN:-}
if [ -z "$tracker_bin" ] || [ ! -x "$tracker_bin" ]; then
    printf '%s\n' 'WX_GNOME_TRACKER_BIN must name the built wx-e2e-gnome-tracker executable' >&2
    exit 2
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-gnome51.XXXXXX")
runtime=$(mktemp -d "${TMPDIR:-/tmp}/wxgnome51.XXXXXX")
chmod 0700 "$runtime"
cleanup() {
    if [ -n "${shell_pid:-}" ]; then
        kill "$shell_pid" 2>/dev/null || true
        wait "$shell_pid" 2>/dev/null || true
    fi
    attempt=0
    while [ "$attempt" -lt 10 ]; do
        if rm -rf -- "$work" "$runtime" 2>/dev/null; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.1
    done
    # D-Bus-activated desktop helpers can finish writing metadata just after
    # their session bus exits. Cleanup is best-effort after the bounded retry;
    # it must not turn a successful compatibility assertion into a failure.
    rm -rf -- "$work" "$runtime" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$work/home/.local/share/gnome-shell/extensions/$uuid" \
    "$work/config" "$work/cache"
cp "$extension_dir/metadata.json" "$extension_dir/extension.js" \
    "$extension_dir/notification-queue.js" "$extension_dir/keyboard-compat.js" \
    "$work/home/.local/share/gnome-shell/extensions/$uuid/"
chmod 0700 "$work" "$work/home" "$work/config" "$work/cache"
chmod 0700 "$work/home/.local/share/gnome-shell/extensions/$uuid"

export HOME="$work/home"
export XDG_CONFIG_HOME="$work/config"
export XDG_CACHE_HOME="$work/cache"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_RUNTIME_DIR="$runtime"
export WAYLAND_DISPLAY=wayexpand-gnome51
export GSETTINGS_BACKEND=dconf

cat >"$work/session.sh" <<'EOF'
#!/bin/sh
set -eu

    uuid=$1
    work=$2
    display=$3
    . "$4/scripts/gnome-extension-dbus.sh"
    . "$4/scripts/gnome-extension-state.sh"
    gsettings set org.gnome.shell disable-user-extensions false
    gsettings set org.gnome.shell enabled-extensions "['$uuid']"
    gsettings set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('xkb', 'de')]"
    gnome-shell --headless --mode=user --wayland --virtual-monitor 800x600 \
        --wayland-display "$display" >"$work/gnome-shell.log" 2>&1 &
    shell_pid=$!
    trap "kill \$shell_pid 2>/dev/null || true; wait \$shell_pid 2>/dev/null || true" EXIT HUP INT TERM

    tries=600
    while ! gnome_shell_extensions_call EnableExtension "$uuid" >"$work/enable.log" 2>&1; do
        tries=$((tries - 1))
        if [ "$tries" -le 0 ] || ! kill -0 "$shell_pid" 2>/dev/null; then
            cat "$work/enable.log" >&2
            cat "$work/gnome-shell.log" >&2
            echo "FAIL: GNOME Shell could not enable $uuid" >&2
            exit 1
        fi
        sleep 0.1
    done

    tries=300
    state=
    while [ "$tries" -gt 0 ]; do
        state=$(gnome_shell_extensions_call GetExtensionInfo "$uuid" 2>/dev/null || true)
        if gnome_extension_state_active "$state"; then
            break
        fi
        tries=$((tries - 1))
        sleep 0.1
    done
    if ! gnome_extension_state_active "$state"; then
        cat "$work/gnome-shell.log" >&2
        echo "FAIL: extension did not reach active state: $state" >&2
        exit 1
    fi

    expected=$(python3 - "$work/home/.local/share/gnome-shell/extensions/$uuid/metadata.json" <<"PY"
import json
import sys
with open(sys.argv[1], encoding="utf-8") as source:
    metadata = json.load(source)
assert "51" in metadata["shell-version"], "metadata omits GNOME 51"
print(metadata["x-wayexpand-build-id"])
PY
)
    live=$(gdbus call --timeout 5 --session --dest org.gnome.Shell \
        --object-path /org/wayexpand/ShellWindowTracker \
        --method org.freedesktop.DBus.Properties.Get \
        org.wayexpand.ShellWindowTracker1 BuildIdentity)
    case "$live" in
        *"$expected"*) echo "PASS: GNOME Shell 51 loaded the expected extension build" ;;
        *) echo "FAIL: live build identity does not match installed files: $live" >&2; exit 1 ;;
    esac

    protocol=$(gdbus introspect --session --dest org.gnome.Shell \
        --object-path /org/wayexpand/ShellWindowTracker)
    for method in Subscribe SubscribeWithTitles GetActiveInputSource \
        GetActiveKeyboardLayout ActivateInputSource; do
        if ! printf "%s\n" "$protocol" | grep -F "$method" >/dev/null; then
            echo "FAIL: GNOME Shell 51 did not export tracker method $method" >&2
            exit 1
        fi
    done
    echo "PASS: GNOME Shell 51 exported the input-source and tracker protocol"

    tracker=$5
    active_source=$("$tracker" --active-source)
    case "$active_source" in
        "active-source xkb "*) echo "PASS: GNOME Shell 51 active-source API: $active_source" ;;
        *) echo "FAIL: unexpected active input source: $active_source" >&2; exit 1 ;;
    esac
    active_layout=$("$tracker" --active-layout)
    case "$active_layout" in
        "active-layout xkb us us "*|"active-layout xkb de de "*) ;;
        *) echo "FAIL: unexpected initial keyboard layout: $active_layout" >&2; exit 1 ;;
    esac
    "$tracker" --activate-source xkb de
    active_layout=$("$tracker" --active-layout)
    case "$active_layout" in
        "active-layout xkb de de "*) echo "PASS: GNOME Shell 51 layout API follows switching to German" ;;
        *) echo "FAIL: GNOME Shell 51 did not report German after activation: $active_layout" >&2; exit 1 ;;
    esac
    "$tracker" --activate-source xkb us
    active_layout=$("$tracker" --active-layout)
    case "$active_layout" in
        "active-layout xkb us us "*) echo "PASS: GNOME Shell 51 layout API follows switching back to US" ;;
        *) echo "FAIL: GNOME Shell 51 did not report US after activation: $active_layout" >&2; exit 1 ;;
    esac
EOF
chmod 0600 "$work/session.sh"
export WAYEXPAND_GNOME_TEST_CLIENTS="$tracker_bin"
dbus-run-session -- sh "$work/session.sh" "$uuid" "$work" \
    "$WAYLAND_DISPLAY" "$project_dir" "$tracker_bin"
