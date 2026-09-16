#!/bin/sh
# Grants permission to use `--source=evdev` (raw keyboard capture, for
# compositors without input-method-v2/virtual-keyboard support such as
# KWin). This is a SEPARATE, explicit, root-requiring step: it is never run
# automatically by install-user.sh or install-release.sh, and evdev capture
# has real security consequences ordinary installation does not.
#
# What this does:
#   1. Installs udev/71-wayexpand-evdev.rules to /etc/udev/rules.d/ and
#      reloads udev rules.
#   2. Adds the invoking (non-root) user to the `input` group.
#
# `input` group membership lets a process read every keystroke typed on
# this system, in any session -- including other users' terminals and
# password fields -- not only keystrokes WayExpand's matcher sees. Read
# docs/SECURITY.md before running this.
#
# Usage:
#   sudo ./scripts/install-evdev-permissions.sh [--dry-run] [--uninstall]
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
rule_src="$project_dir/udev/71-wayexpand-evdev.rules"
rule_dest="/etc/udev/rules.d/71-wayexpand-evdev.rules"
dry_run=0
do_uninstall=0

for argument in "$@"; do
    case "$argument" in
        --dry-run)
            dry_run=1
            ;;
        --uninstall)
            do_uninstall=1
            ;;
        --help|-h)
            printf '%s\n' "usage: sudo $0 [--dry-run] [--uninstall]"
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
    id -nG "$target_user" 2>/dev/null | tr ' ' '\n' | grep -qx input
}

if [ "$dry_run" -eq 0 ] && ! getent group input >/dev/null 2>&1; then
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
    if is_member; then
        printf '%s\n' "  - remove $target_user from the \`input\` group"
    else
        printf '%s\n' "  - leave $target_user out of the \`input\` group (already not a member)"
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
    if is_member; then
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
    exit 0
fi

rule_installed=0
if [ -e "$rule_dest" ] && cmp -s "$rule_src" "$rule_dest"; then
    rule_installed=1
fi

printf '%s\n' "This will:"
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
printf '\n%s\n' "The \`input\` group can read every keystroke typed on this system, in any"
printf '%s\n' "session -- not only keystrokes WayExpand matches against. See"
printf '%s\n' "docs/SECURITY.md before continuing."

if [ "$dry_run" -eq 1 ]; then
    printf '\n%s\n' "(dry run; no changes made)"
    exit 0
fi

if [ "$rule_installed" -eq 0 ]; then
    install -Dm644 "$rule_src" "$rule_dest"
    if command -v udevadm >/dev/null 2>&1; then
        udevadm control --reload
        udevadm trigger --subsystem-match=input
    fi
    printf '%s\n' "Installed $rule_dest"
fi

if ! is_member; then
    usermod -aG input "$target_user"
    printf '%s\n' "Added $target_user to the \`input\` group."
fi

printf '%s\n' "Log out and back in (or reboot) for group membership to take effect."
printf '%s\n' "If \`wayexpand doctor\` still reports a permission problem after that, your"
printf '%s\n' "systemd --user manager likely did not restart and is still running with the"
printf '%s\n' "old group list -- run \`loginctl terminate-user $target_user\` (ends all"
printf '%s\n' "sessions for that user) or reboot, then check again."
printf '%s\n' "Then verify with: wayexpand doctor"
printf '%s\n' "To undo later: sudo $0 --uninstall"
