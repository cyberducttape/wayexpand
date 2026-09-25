#!/bin/sh
# Grants permission to use `--source=evdev` (raw keyboard capture, for
# compositors without input-method-v2/virtual-keyboard support such as
# KWin). This is a SEPARATE, explicit, root-requiring step: it is never run
# automatically by install-user.sh or install-release.sh, and evdev capture
# has real security consequences ordinary installation does not.
#
# What this does:
#   1. Installs exactly one selected udev policy to /etc/udev/rules.d/ and
#      removes the other WayExpand evdev policy if present.
#   2. For --access=input-group only, adds the invoking non-root user to the
#      `input` group.
#
# This is the current legacy/simple access model. The WayExpand rule is scoped
# to udev keyboard-class event nodes, but `input` group membership itself may
# be broader on a given distribution and can grant raw access to input event
# devices beyond WayExpand's matcher. It is not the long-term preferred
# architecture; see docs/EVDEV_ACCESS_DESIGN.md for the active-seat ACL and
# device-broker investigation. Read SECURITY.md before running this.
#
# Usage:
#   sudo ./scripts/install-evdev-permissions.sh [--access=input-group|active-seat]
#       [--dry-run] [--uninstall]
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
if [ -f /usr/share/wayexpand/udev/71-wayexpand-evdev.rules ]; then
    # Distro package layout: policies are deliberately outside udev's active
    # rules directory until this helper is run explicitly.
    rule_src=/usr/share/wayexpand/udev/71-wayexpand-evdev.rules
    uaccess_rule_src=/usr/share/wayexpand/udev/69-wayexpand-evdev-uaccess.rules
else
    # Source tree and release archive layout.
    rule_src="$project_dir/udev/71-wayexpand-evdev.rules"
    uaccess_rule_src="$project_dir/udev/69-wayexpand-evdev-uaccess.rules"
fi
rule_dest=${WAYEXPAND_EVDEV_RULE_DEST:-/etc/udev/rules.d/71-wayexpand-evdev.rules}
uaccess_rule_dest=${WAYEXPAND_EVDEV_UACCESS_RULE_DEST:-/etc/udev/rules.d/69-wayexpand-evdev-uaccess.rules}
state_file=${WAYEXPAND_EVDEV_STATE_FILE:-/var/lib/wayexpand/evdev-permissions.state}
access_mode=active-seat
access_explicit=0
dry_run=0
do_uninstall=0

for argument in "$@"; do
    case "$argument" in
        --access=input-group|--access=active-seat)
            access_mode=${argument#--access=}
            access_explicit=1
            ;;
        --dry-run)
            dry_run=1
            ;;
        --uninstall)
            do_uninstall=1
            ;;
        --help|-h)
            printf '%s\n' "usage: sudo $0 [--access=input-group|active-seat] [--dry-run] [--uninstall]"
            printf '%s\n' "  --access=active-seat  logind/uaccess ACL; no group membership change (default)"
            printf '%s\n' "  --access=input-group  broad legacy input-group grant"
            printf '%s\n' "  --dry-run    print what would change without changing anything"
            printf '%s\n' "  --uninstall  remove the udev rule and the input group grant instead"
            exit 0
            ;;
        *)
            printf '%s\n' "error: unknown option $argument; try --help" >&2
            exit 2
            ;;
    esac
done

if [ ! -f "$rule_src" ]; then
    printf '%s\n' "error: $rule_src not found; run this from the project tree or a release tarball" >&2
    exit 1
fi
if [ "$access_mode" = active-seat ] && [ ! -f "$uaccess_rule_src" ]; then
    printf '%s\n' "error: $uaccess_rule_src not found" >&2
    exit 1
fi

# --dry-run may be run without sudo, purely to preview; every other path
# (installing for real, or --uninstall) requires root since it changes
# system-wide udev rules and group membership.
if [ "$dry_run" -eq 0 ] || [ "$do_uninstall" -eq 1 ]; then
    if [ "$(id -u)" -ne 0 ]; then
        printf '%s\n' "error: this installs a system-wide udev rule and changes group membership; run it with sudo" >&2
        exit 1
    fi
fi

if [ "$(id -u)" -eq 0 ]; then
    target_user=${SUDO_USER:-}
    if [ -z "$target_user" ]; then
        printf '%s\n' "error: could not determine which user to grant access to; run via sudo as your desktop user, not as root directly" >&2
        exit 1
    fi
else
    target_user=$(id -un)
fi

is_member() {
    id -nG "$target_user" 2>/dev/null | tr '[:space:]' '\n' | grep -qx input
}

state_added_input_group=0
state_user=
state_mode=
if [ -r "$state_file" ]; then
    while IFS='=' read -r key value; do
        case "$key" in
            added_input_group) state_added_input_group=$value ;;
            target_user) state_user=$value ;;
            access_mode) state_mode=$value ;;
        esac
    done <"$state_file"
fi

# Keep the safe active-seat default, while allowing an uninstall to clean up
# an older explicitly selected input-group installation.
if [ "$do_uninstall" -eq 1 ] && [ "$access_explicit" -eq 0 ] && [ -n "$state_mode" ]; then
    access_mode=$state_mode
fi

if [ "$access_mode" = input-group ] && [ "$dry_run" -eq 0 ] && ! getent group input >/dev/null 2>&1; then
    printf '%s\n' "error: the \`input\` group does not exist on this system; cannot continue" >&2
    exit 1
fi

if [ "$do_uninstall" -eq 1 ]; then
    printf '%s\n' "This will:"
    if [ -e "$rule_dest" ]; then
        printf '%s\n' "  - remove $rule_dest"
    else
        printf '%s\n' "  - leave $rule_dest alone (not present)"
    fi
    if [ -e "$uaccess_rule_dest" ]; then
        printf '%s\n' "  - remove $uaccess_rule_dest"
    else
        printf '%s\n' "  - leave $uaccess_rule_dest alone (not present)"
    fi
    if [ "$access_mode" = input-group ] && [ "$state_added_input_group" -eq 1 ] \
        && [ "$state_user" = "$target_user" ] && is_member; then
        printf '%s\n' "  - remove $target_user from the \`input\` group"
    else
        printf '%s\n' "  - leave $target_user's existing \`input\` group membership unchanged"
    fi
    if [ "$dry_run" -eq 1 ]; then
        printf '%s\n' "(dry run; no changes made)"
        exit 0
    fi
    if [ -e "$rule_dest" ]; then
        rm -f -- "$rule_dest"
        command -v udevadm >/dev/null 2>&1 && udevadm control --reload
        printf '%s\n' "Removed $rule_dest"
    fi
    if [ -e "$uaccess_rule_dest" ]; then
        rm -f -- "$uaccess_rule_dest"
        command -v udevadm >/dev/null 2>&1 && udevadm control --reload
        printf '%s\n' "Removed $uaccess_rule_dest"
    fi
    if [ "$access_mode" = input-group ] && [ "$state_added_input_group" -eq 1 ] \
        && [ "$state_user" = "$target_user" ] && is_member; then
        if command -v gpasswd >/dev/null 2>&1; then
            gpasswd -d "$target_user" input >/dev/null
        else
            deluser "$target_user" input >/dev/null
        fi
        printf '%s\n' "Removed $target_user from the \`input\` group."
        printf '%s\n' "Log out and back in for the removal to take effect."
        printf '%s\n' "If it still applies after that, your systemd --user manager likely did not"
        printf '%s\n' "restart and is still running with the old group list -- run"
        printf '%s\n' "\`loginctl terminate-user $target_user\` (ends all sessions for that user) or"
        printf '%s\n' "reboot, then check again."
    fi
    if [ -e "$state_file" ]; then
        rm -f -- "$state_file"
    fi
    exit 0
fi

rule_installed=0
active_rule_installed=0
if [ "$access_mode" = input-group ] && [ -e "$rule_dest" ] && cmp -s "$rule_src" "$rule_dest"; then
    rule_installed=1
fi
if [ "$access_mode" = active-seat ] && [ -e "$uaccess_rule_dest" ] && cmp -s "$uaccess_rule_src" "$uaccess_rule_dest"; then
    active_rule_installed=1
fi

printf '%s\n' "This will:"
if [ "$access_mode" = active-seat ]; then
    if [ -e "$rule_dest" ]; then
        printf '%s\n' "  - remove $rule_dest so it cannot override active-seat ACLs"
    fi
    if [ "$active_rule_installed" -eq 1 ]; then
        printf '%s\n' "  - keep $uaccess_rule_dest (already installed, unchanged)"
    else
        printf '%s\n' "  - install $uaccess_rule_dest and reload udev rules"
    fi
    if [ "$state_added_input_group" -eq 1 ] && [ "$state_user" = "$target_user" ] && is_member; then
        printf '%s\n' "  - remove the input-group membership previously added by WayExpand"
    else
        printf '%s\n' "  - leave pre-existing input-group membership unchanged"
    fi
else
    if [ -e "$uaccess_rule_dest" ]; then
        printf '%s\n' "  - remove $uaccess_rule_dest so the input-group model is unambiguous"
    fi
    if [ "$rule_installed" -eq 1 ]; then
        printf '%s\n' "  - keep $rule_dest (already installed, unchanged)"
    else
        printf '%s\n' "  - install $rule_dest and reload udev rules"
    fi
    if is_member; then
        printf '%s\n' "  - keep $target_user in the \`input\` group (already a member)"
    else
        printf '%s\n' "  - add $target_user to the \`input\` group"
    fi
fi
if [ "$access_mode" = input-group ]; then
    printf '\n%s\n' "This installs a keyboard-event udev rule, but the \`input\` group may"
    printf '%s\n' "also read other raw input event devices on this system. See"
    printf '%s\n' "SECURITY.md and docs/EVDEV_ACCESS_DESIGN.md before continuing."
else
    printf '\n%s\n' "Active-seat mode relies on systemd-logind uaccess ACLs. Verify the"
    printf '%s\n' "ACL is present for the current seat before starting WayExpand."
fi

if [ "$dry_run" -eq 1 ]; then
    printf '\n%s\n' "(dry run; no changes made)"
    exit 0
fi

if [ "$access_mode" = active-seat ] && [ "$state_added_input_group" -eq 1 ] \
    && [ "$state_user" = "$target_user" ] && is_member; then
    if command -v gpasswd >/dev/null 2>&1; then
        gpasswd -d "$target_user" input >/dev/null
    else
        deluser "$target_user" input >/dev/null
    fi
    state_added_input_group=0
    printf '%s\n' "Removed the input-group membership previously added by WayExpand."
fi

udev_changed=0
if [ "$access_mode" = active-seat ] && [ -e "$rule_dest" ]; then
    rm -f -- "$rule_dest"
    udev_changed=1
    printf '%s\n' "Removed $rule_dest"
fi
if [ "$access_mode" = input-group ] && [ -e "$uaccess_rule_dest" ]; then
    rm -f -- "$uaccess_rule_dest"
    udev_changed=1
    printf '%s\n' "Removed $uaccess_rule_dest"
fi

if [ "$access_mode" = active-seat ] && [ "$active_rule_installed" -eq 0 ]; then
    install -Dm644 "$uaccess_rule_src" "$uaccess_rule_dest"
    udev_changed=1
    printf '%s\n' "Installed $uaccess_rule_dest"
elif [ "$access_mode" = input-group ] && [ "$rule_installed" -eq 0 ]; then
    install -Dm644 "$rule_src" "$rule_dest"
    udev_changed=1
    printf '%s\n' "Installed $rule_dest"
fi

if [ "$udev_changed" -eq 1 ] && command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload
    udevadm trigger --subsystem-match=input
fi

if [ "$access_mode" = input-group ] && ! is_member; then
    usermod -aG input "$target_user"
    state_added_input_group=1
    printf '%s\n' "Added $target_user to the \`input\` group."
fi

install -d -m 0755 "$(dirname -- "$state_file")"
umask 077
{
    printf 'access_mode=%s\n' "$access_mode"
    printf 'target_user=%s\n' "$target_user"
    printf 'added_input_group=%s\n' "$state_added_input_group"
} >"$state_file"

if [ "$access_mode" = active-seat ]; then
    printf '%s\n' "Active-seat ACLs apply after udev reload/trigger and seat activation."
else
    printf '%s\n' "Log out and back in (or reboot) for group membership to take effect."
fi
printf '%s\n' "If \`wayexpand doctor\` still reports a permission problem after that, your"
printf '%s\n' "systemd --user manager likely did not restart and is still running with the"
printf '%s\n' "old group list -- run \`loginctl terminate-user $target_user\` (ends all"
printf '%s\n' "sessions for that user) or reboot, then check again."
printf '%s\n' "Then verify with: wayexpand doctor"
printf '%s\n' "To undo later: sudo $0 --uninstall"
