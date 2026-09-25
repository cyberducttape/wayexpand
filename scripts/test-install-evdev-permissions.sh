#!/bin/sh
# CI-safe checks for install-evdev-permissions.sh: everything that does not
# require root or touch real system state (installing the rule for real,
# and modifying group membership, both need root and are not exercised
# here -- a human running this by hand with sudo is the real test for
# those).
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
script="$project_dir/scripts/install-evdev-permissions.sh"

"$script" --help >/dev/null

out=$("$script" --dry-run)
printf '%s' "$out" | grep -F 'This will:' >/dev/null
printf '%s' "$out" | grep -F '(dry run; no changes made)' >/dev/null
printf '%s' "$out" | grep -F 'Active-seat mode relies on systemd-logind' >/dev/null
seat_out=$($script --access=active-seat --dry-run)
printf '%s' "$seat_out" | grep -F 'active-seat' >/dev/null
printf '%s' "$seat_out" | grep -F 'leave pre-existing input-group membership unchanged' >/dev/null

empty_dir=
policy_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-evdev-policy-test.XXXXXX")
trap 'rm -rf "$policy_dir" "$empty_dir"' EXIT INT TERM
group_rule="$policy_dir/71-wayexpand-evdev.rules"
seat_rule="$policy_dir/69-wayexpand-evdev-uaccess.rules"
state_file="$policy_dir/evdev-permissions.state"
printf '%s\n' 'stale input-group rule' >"$group_rule"
exclusive_seat_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    "$script" --access=active-seat --dry-run)
printf '%s' "$exclusive_seat_out" | grep -F "remove $group_rule" >/dev/null
printf '%s\n' 'stale active-seat rule' >"$seat_rule"
exclusive_group_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    "$script" --access=input-group --dry-run)
printf '%s' "$exclusive_group_out" | grep -F "remove $seat_rule" >/dev/null

# A recorded membership grant is owned by WayExpand and is removed when an
# existing installation is migrated to active-seat. An unrecorded membership
# is deliberately left alone.
printf '%s\n' 'access_mode=input-group' "target_user=$(id -un)" 'added_input_group=1' >"$state_file"
owned_migration_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    WAYEXPAND_EVDEV_STATE_FILE="$state_file" \
    "$script" --access=active-seat --dry-run)
# The result is intentionally dependent on the runner's actual group list:
# remove a membership only when it exists, otherwise leave it untouched. Do
# not make CI depend on whether its image happens to define or grant `input`.
case "$owned_migration_out" in
    *'remove the input-group membership previously added by WayExpand'*|\
    *'leave pre-existing input-group membership unchanged'*)
        ;;
    *)
        printf '%s\n' "$owned_migration_out" >&2
        printf '%s\n' 'unexpected owned-membership migration output' >&2
        exit 1
        ;;
esac

printf '%s\n' 'access_mode=input-group' "target_user=$(id -un)" 'added_input_group=0' >"$state_file"
unowned_migration_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    WAYEXPAND_EVDEV_STATE_FILE="$state_file" \
    "$script" --access=active-seat --dry-run)
printf '%s' "$unowned_migration_out" | grep -F 'leave pre-existing input-group membership unchanged' >/dev/null

if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' "skipping non-root-rejection checks: already running as root" >&2
else
    if "$script" >/dev/null 2>&1; then
        printf '%s\n' "install-evdev-permissions.sh accepted a non-root apply" >&2
        exit 1
    fi
    if "$script" --uninstall >/dev/null 2>&1; then
        printf '%s\n' "install-evdev-permissions.sh accepted a non-root --uninstall" >&2
        exit 1
    fi
fi

if "$script" --bogus-flag >/dev/null 2>&1; then
    printf '%s\n' "install-evdev-permissions.sh accepted an unknown flag" >&2
    exit 1
fi

# Confirm it fails clearly when not run from the project tree / a release
# layout (no udev/71-wayexpand-evdev.rules next to it), instead of a bare
# "No such file" further down.
empty_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-evdev-perm-test.XXXXXX")
mkdir -p "$empty_dir/scripts"
cp "$script" "$empty_dir/scripts/"
if "$empty_dir/scripts/install-evdev-permissions.sh" --dry-run >"$empty_dir/out" 2>&1; then
    printf '%s\n' "install-evdev-permissions.sh ran without its udev rule file present" >&2
    exit 1
fi
grep -F 'not found' "$empty_dir/out" >/dev/null

printf '%s\n' "install-evdev-permissions.sh test passed"
