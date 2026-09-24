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
ibus_component_dir="$HOME/.local/share/ibus/component"
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
evdev_rule_dest=${WAYEXPAND_EVDEV_RULE_DEST:-/etc/udev/rules.d/71-wayexpand-evdev.rules}
evdev_uaccess_rule_dest=${WAYEXPAND_EVDEV_UACCESS_RULE_DEST:-/etc/udev/rules.d/69-wayexpand-evdev-uaccess.rules}

if command -v systemctl >/dev/null 2>&1; then
    for service_name in wayexpand.service wayexpand-input-method.service wayexpand-evdev.service; do
        if systemctl --user is-enabled "$service_name" >/dev/null 2>&1 \
            || systemctl --user is-active "$service_name" >/dev/null 2>&1; then
            printf '%s\n' "Stopping and disabling $service_name"
            systemctl --user disable --now "$service_name" >/dev/null 2>&1 || true
        fi
    done
fi

for binary in wayexpand-daemon wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    if [ -e "$bin_dir/$binary" ]; then
        rm -f -- "$bin_dir/$binary"
        printf '%s\n' "Removed $bin_dir/$binary"
    fi
done
if [ -e "$ibus_component_dir/wayexpand.xml" ]; then
    rm -f -- "$ibus_component_dir/wayexpand.xml"
    printf '%s\n' "Removed $ibus_component_dir/wayexpand.xml"
fi
if [ -e "$ibus_component_dir/wayexpand-ibus.xml" ]; then
    rm -f -- "$ibus_component_dir/wayexpand-ibus.xml"
    printf '%s\n' "Removed $ibus_component_dir/wayexpand-ibus.xml"
fi

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

raw_input_configured=0
current_user=$(id -un)
user_groups=${WAYEXPAND_UNINSTALL_GROUPS:-$(id -nG)}
if printf '%s\n' "$user_groups" | tr ' ' '\n' | grep -qx input; then
    raw_input_configured=1
fi
if [ -e "$evdev_rule_dest" ] || [ -e "$evdev_uaccess_rule_dest" ]; then
    raw_input_configured=1
fi

printf '%s\n' "WayExpand user files were removed."

if [ "$raw_input_configured" -eq 1 ]; then
    printf '\n%s\n' "WARNING: raw-input privileges are still configured:"
    if printf '%s\n' "$user_groups" | tr ' ' '\n' | grep -qx input; then
        printf '%s\n' "  - user $current_user is a member of the input group"
    fi
    if [ -e "$evdev_rule_dest" ]; then
        printf '%s\n' "  - WayExpand input-group udev rule is installed: $evdev_rule_dest"
    fi
    if [ -e "$evdev_uaccess_rule_dest" ]; then
        printf '%s\n' "  - WayExpand active-seat udev rule is installed: $evdev_uaccess_rule_dest"
    fi

    if command -v wayexpand-install-evdev-access >/dev/null 2>&1; then
        cleanup_command="sudo wayexpand-install-evdev-access --uninstall"
    elif [ -x "$script_dir/install-evdev-permissions.sh" ]; then
        cleanup_command="sudo $script_dir/install-evdev-permissions.sh --uninstall"
    else
        cleanup_command="sudo /path/to/install-evdev-permissions.sh --uninstall"
    fi
    printf '\n%s\n' "Remove them with:"
    printf '%s\n' "  $cleanup_command"
fi
