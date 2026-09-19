#!/bin/sh
# Installs a downloaded WayExpand release tarball for the current user.
# Run this from inside the extracted tarball directory
# (the one containing bin/, systemd/, desktop/, and expansions.toml).
set -eu

release_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
enable_service=0
service_name=
for argument in "$@"; do
    case "$argument" in
        --enable)
            enable_service=1
            ;;
        --service=wayexpand.service)
            service_name=wayexpand.service
            ;;
        --service=wayexpand-input-method.service)
            service_name=wayexpand-input-method.service
            ;;
        --service=wayexpand-evdev.service)
            service_name=wayexpand-evdev.service
            ;;
        --help|-h)
            printf '%s\n' "usage: $0 [--enable] [--service=wayexpand.service|wayexpand-input-method.service|wayexpand-evdev.service]"
            printf '%s\n' "  --enable   daemon-reload and enable the selected user service"
            printf '%s\n' "  --service  select the service when --enable is used"
            exit 0
            ;;
        *)
            printf '%s\n' "error: unknown option $argument; try --help" >&2
            exit 2
            ;;
    esac
done
if [ "$(id -u)" -eq 0 ]; then
    if [ -n "${SUDO_USER:-}" ]; then
        printf '%s\n' "error: do not run the user installer with sudo; run it as $SUDO_USER" >&2
    else
        printf '%s\n' "error: this is a per-user installer; run it as your desktop user, not root" >&2
    fi
    exit 1
fi
for binary in wayexpand-daemon wayexpand wayexpand-ui wayexpand-gui; do
    if [ ! -x "$release_dir/bin/$binary" ]; then
        printf '%s\n' "error: $release_dir/bin/$binary not found" >&2
        printf '%s\n' "run this script from inside the extracted release tarball" >&2
        exit 1
    fi
done

bin_dir="$HOME/.local/bin"
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$HOME/.local/share/applications"
metainfo_dir="$HOME/.local/share/metainfo"
man_dir="$HOME/.local/share/man/man1"

install -Dm755 "$release_dir/bin/wayexpand-daemon" "$bin_dir/wayexpand-daemon"
install -Dm755 "$release_dir/bin/wayexpand" "$bin_dir/wayexpand"
install -Dm755 "$release_dir/bin/wayexpand-ui" "$bin_dir/wayexpand-ui"
install -Dm755 "$release_dir/bin/wayexpand-gui" "$bin_dir/wayexpand-gui"
install -Dm644 "$release_dir/systemd/wayexpand.service" \
    "$unit_dir/wayexpand.service"
install -Dm644 "$release_dir/systemd/wayexpand-input-method.service" \
    "$unit_dir/wayexpand-input-method.service"
install -Dm644 "$release_dir/systemd/wayexpand-evdev.service" \
    "$unit_dir/wayexpand-evdev.service"
install -Dm644 "$release_dir/desktop/wayexpand.desktop" \
    "$application_dir/wayexpand.desktop"
if [ -f "$release_dir/io.github.itchyitchy123.WayExpand.metainfo.xml" ]; then
    install -Dm644 "$release_dir/io.github.itchyitchy123.WayExpand.metainfo.xml" \
        "$metainfo_dir/io.github.itchyitchy123.WayExpand.metainfo.xml"
fi
if [ -f "$release_dir/docs/wayexpand.1" ]; then
    install -Dm644 "$release_dir/docs/wayexpand.1" "$man_dir/wayexpand.1"
fi

icon_base="$HOME/.local/share/icons/hicolor"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    icon_src="$release_dir/icons/hicolor/$size/apps/wayexpand.png"
    if [ -f "$icon_src" ]; then
        install -Dm644 "$icon_src" "$icon_base/$size/apps/wayexpand.png"
    fi
done

config_path="$config_dir/expansions.toml"
if [ -e "$config_path" ] || [ -L "$config_path" ]; then
    printf '%s\n' "Keeping existing configuration: $config_path"
elif [ -f "$release_dir/expansions.toml" ]; then
    install -Dm600 "$release_dir/expansions.toml" "$config_path"
    printf '%s\n' "Installed example configuration: $config_path"
fi

printf '%s\n' "Installed binaries in $bin_dir"
printf '%s\n' "Installed user units in $unit_dir"
printf '%s\n' "Installed desktop entry in $application_dir"
if [ -d "$release_dir/icons" ]; then
    printf '%s\n' "Installed application icon in $icon_base"
fi
printf '%s\n' "Next steps:"
printf '%s\n' "  export PATH=\"$bin_dir:\$PATH\""
printf '%s\n' "  systemctl --user daemon-reload"
printf '%s\n' "  wayexpand doctor"
printf '%s\n' "  wayexpand backend select --explain"
printf '%s\n' "  systemctl --user enable --now wayexpand-evdev.service (normal route, after evdev permissions)"
printf '%s\n' "  systemctl --user enable --now wayexpand-input-method.service (explicit experimental opt-in; may lose non-text keys)"
printf '%s\n' "  wayexpand-gui"
printf '%s\n' "If \`doctor\` reports no input-method-v2/virtual-keyboard support (for"
printf '%s\n' "example on KWin/KDE Plasma), read SECURITY.md and consider:"
printf '%s\n' "  sudo ./scripts/install-evdev-permissions.sh --dry-run"
printf '%s\n' "  systemctl --user enable --now wayexpand-evdev.service"
printf '%s\n' "To remove this installation later, run scripts/uninstall-user.sh."

if [ "$enable_service" -eq 1 ]; then
    if [ -z "$service_name" ]; then
        printf '%s\n' "error: --enable requires an explicit --service selection" >&2
        printf '%s\n' "choose wayexpand-input-method.service only after accepting its key pass-through limitation, or wayexpand-evdev.service for the normal route" >&2
        exit 2
    fi
    "$bin_dir/wayexpand" validate "$config_path"
    if ! command -v systemctl >/dev/null 2>&1; then
        printf '%s\n' "error: systemctl is required for --enable" >&2
        exit 127
    fi
    printf '%s\n' "Enabling user service: $service_name"
    systemctl --user daemon-reload
    systemctl --user enable --now "$service_name"
    printf '%s\n' "Enabled $service_name"
fi
