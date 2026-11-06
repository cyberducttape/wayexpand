#!/bin/sh
# CI-safe checks for install-evdev-permissions.sh: everything that does not
# require root or touch real system state (installing the rule for real,
# and modifying group membership, both need root and are not exercised
# here -- a human running this by hand with sudo is the real test for
# those).
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
script="$project_dir/scripts/install-evdev-permissions.sh"

assert_contains() {
    label=$1
    expected=$2
    actual=$3
    case "$actual" in
        *"$expected"*) ;;
        *)
            printf '%s\n' "assertion failed: $label" >&2
            printf '%s\n' "expected substring: $expected" >&2
            printf '%s\n' "actual output:" >&2
            printf '%s\n' "$actual" >&2
            exit 1
            ;;
    esac
}

"$script" --help >/dev/null

# Simulate a root invocation in a container without SUDO_USER. No privileged
# operation runs because this is a dry run; the fake id only exercises the
# script's root/user-selection branch.
fake_bin=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-evdev-root-preview.XXXXXX")
trap 'rm -rf "$fake_bin"' EXIT INT TERM
cat >"$fake_bin/id" <<'EOF'
#!/bin/sh
if [ "$#" -eq 1 ] && [ "$1" = -u ]; then
    printf '0\n'
else
    exec /usr/bin/id "$@"
fi
EOF
chmod 0755 "$fake_bin/id"
root_preview=$(env PATH="$fake_bin:$PATH" SUDO_USER= "$script" --dry-run)
assert_contains root-without-sudo-user '(dry run; no changes made)' "$root_preview"
assert_contains root-without-sudo-user-no-account-change 'no account membership change' "$root_preview"
if env PATH="$fake_bin:$PATH" SUDO_USER= "$script" --access=input-group --dry-run >/dev/null 2>&1; then
    printf '%s\n' 'root input-group preview accepted a missing target user' >&2
    exit 1
fi

out=$("$script" --dry-run)
assert_contains default-header 'This will:' "$out"
assert_contains default-dry-run '(dry run; no changes made)' "$out"
assert_contains default-active-seat 'Active-seat mode relies on systemd-logind' "$out"
seat_out=$("$script" --access=active-seat --dry-run)
assert_contains explicit-active-seat 'Active-seat mode' "$seat_out"
case "$seat_out" in
    *'leave pre-existing input-group membership unchanged'*|\
    *'no account membership change'*)
        ;;
    *)
        printf '%s\n' "$seat_out" >&2
        printf '%s\n' 'active-seat preview did not explain its membership behavior' >&2
        exit 1
        ;;
esac

# An explicit account is accepted and validated for a group-mode preview.
preview_user=$(id -un)
group_preview=$("$script" --access=input-group --user "$preview_user" --dry-run)
assert_contains group-preview-user "$preview_user" "$group_preview"
if "$script" --access=input-group --dry-run >/dev/null 2>&1; then
    printf '%s\n' 'input-group preview accepted a missing target user' >&2
    exit 1
fi
if "$script" --dry-run --user wayexpand-user-that-does-not-exist >/dev/null 2>&1; then
    printf '%s\n' 'evdev helper accepted a nonexistent target user' >&2
    exit 1
fi
if [ "$(id -u)" -eq 0 ]; then
    root_preview=$(env SUDO_USER= "$script" --dry-run)
    assert_contains root-preview '(dry run; no changes made)' "$root_preview"
fi

empty_dir=
policy_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-evdev-policy-test.XXXXXX")
trap 'rm -rf "$policy_dir" "$empty_dir" "$fake_bin"' EXIT INT TERM
group_rule="$policy_dir/71-wayexpand-evdev.rules"
seat_rule="$policy_dir/69-wayexpand-evdev-uaccess.rules"
state_file="$policy_dir/evdev-permissions.state"
printf '%s\n' 'stale input-group rule' >"$group_rule"
exclusive_seat_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    "$script" --access=active-seat --dry-run)
assert_contains remove-group-rule "remove $group_rule" "$exclusive_seat_out"
printf '%s\n' 'stale active-seat rule' >"$seat_rule"
exclusive_group_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    "$script" --access=input-group --user "$preview_user" --dry-run)
assert_contains remove-seat-rule "remove $seat_rule" "$exclusive_group_out"

# A recorded membership grant is owned by WayExpand and is removed when an
# existing installation is migrated to active-seat. An unrecorded membership
# is deliberately left alone.
printf '%s\n' 'access_mode=input-group' "target_user=$(id -un)" 'added_input_group=1' >"$state_file"
owned_migration_out=$(WAYEXPAND_EVDEV_RULE_DEST="$group_rule" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$seat_rule" \
    WAYEXPAND_EVDEV_STATE_FILE="$state_file" \
    "$script" --access=active-seat --user "$(id -un)" --dry-run)
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
    "$script" --access=active-seat --user "$(id -un)" --dry-run)
assert_contains unowned-membership 'leave pre-existing input-group membership unchanged' "$unowned_migration_out"

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
assert_contains missing-rule 'not found' "$(cat "$empty_dir/out")"

if [ "$(id -u)" -eq 0 ]; then
    # Cleanup must remain possible after the source/package files are gone.
    stale_group="$empty_dir/stale-group.rules"
    stale_seat="$empty_dir/stale-seat.rules"
    stale_state="$empty_dir/stale-state"
    printf '%s\n' 'installed rule' >"$stale_group"
    printf '%s\n' 'installed rule' >"$stale_seat"
    WAYEXPAND_EVDEV_RULE_DEST="$stale_group" \
    WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$stale_seat" \
    WAYEXPAND_EVDEV_STATE_FILE="$stale_state" \
    "$empty_dir/scripts/install-evdev-permissions.sh" --uninstall --access=active-seat \
        >"$empty_dir/uninstall-out"
    [ ! -e "$stale_group" ]
    [ ! -e "$stale_seat" ]
else
    printf '%s\n' 'skipping source-free uninstall check: requires root' >&2
fi

printf '%s\n' "install-evdev-permissions.sh test passed"
