#!/bin/sh
# Removes a per-user WayExpand installation made by install-user.sh or
# install-release.sh. Configuration is kept unless --purge is given.
set -eu

purge_config=0
for argument in "$@"; do
    case "$argument" in
        --purge)
            purge_config=1
            ;;
        --help|-h)
            printf '%s\n' "usage: $0 [--purge]"
            printf '%s\n' "  --purge   also remove the configuration directory and its snippets"
            exit 0
            ;;
        *)
            printf '%s\n' "error: unknown option $argument; try --help" >&2
            exit 2
            ;;
    esac
done
if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' "error: this is a per-user uninstaller; run it as your desktop user, not root" >&2
    exit 1
fi

bin_dir="$HOME/.local/bin"
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$HOME/.local/share/applications"
metainfo_dir="$HOME/.local/share/metainfo"
man_dir="$HOME/.local/share/man/man1"

if command -v systemctl >/dev/null 2>&1; then
    for service_name in wayexpand.service wayexpand-input-method.service wayexpand-evdev.service; do
        if systemctl --user is-enabled "$service_name" >/dev/null 2>&1 \
            || systemctl --user is-active "$service_name" >/dev/null 2>&1; then
            printf '%s\n' "Stopping and disabling $service_name"
            systemctl --user disable --now "$service_name" >/dev/null 2>&1 || true
        fi
    done
fi

for binary in wayexpand-daemon wayexpand wayexpand-ui wayexpand-gui; do
    if [ -e "$bin_dir/$binary" ]; then
        rm -f -- "$bin_dir/$binary"
        printf '%s\n' "Removed $bin_dir/$binary"
    fi
done

for unit in wayexpand.service wayexpand-input-method.service wayexpand-evdev.service; do
    if [ -e "$unit_dir/$unit" ]; then
        rm -f -- "$unit_dir/$unit"
        printf '%s\n' "Removed $unit_dir/$unit"
    fi
done
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload || true
fi

if [ -e "$application_dir/wayexpand.desktop" ]; then
    rm -f -- "$application_dir/wayexpand.desktop"
    printf '%s\n' "Removed $application_dir/wayexpand.desktop"
fi

for installed_file in \
    "$metainfo_dir/io.github.itchyitchy123.WayExpand.metainfo.xml" \
    "$man_dir/wayexpand.1"; do
    if [ -e "$installed_file" ]; then
        rm -f -- "$installed_file"
        printf '%s\n' "Removed $installed_file"
    fi
done

icon_base="$HOME/.local/share/icons/hicolor"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    icon_path="$icon_base/$size/apps/wayexpand.png"
    if [ -e "$icon_path" ]; then
        rm -f -- "$icon_path"
    fi
done

if [ "$purge_config" -eq 1 ]; then
    if [ -e "$config_dir" ]; then
        rm -rf -- "$config_dir"
        printf '%s\n' "Removed $config_dir"
    fi
else
    if [ -e "$config_dir" ]; then
        printf '%s\n' "Kept configuration: $config_dir (rerun with --purge to remove it)"
    fi
fi

printf '%s\n' "WayExpand has been uninstalled."
