#!/bin/sh
# Commands that only inspect a configuration must never rewrite it. Loading a
# file with legacy (ID-less) snippets through the adopting loader migrates it
# in place, replacing comments and formatting with canonical TOML; validate,
# show, diff, check, restore review and profile listing must leave every
# input byte-for-byte intact and create no lock files beside it.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project_dir/target/debug/wayexpand"}
if [ ! -x "$binary" ]; then
    printf '%s\n' "error: WayExpand CLI is not executable: $binary" >&2
    exit 1
fi

test_dir=$(mktemp -d)
trap 'rm -rf -- "$test_dir"' EXIT INT TERM
chmod 0700 "$test_dir"
library="$test_dir/library"
mkdir -m 0700 "$library" "$library/profiles"

write_legacy() {
    cat >"$1" <<EOF
# $2: a hand-written comment the user wants to keep
[[expansion]]
trigger   = ":$2"
replacement = "kept as written" # inline comment
EOF
    chmod 0600 "$1"
}

config="$library/expansions.toml"
backup="$library/expansions.toml.bak"
other="$library/other.toml"
profile="$library/profiles/work.toml"
write_legacy "$config" active
write_legacy "$backup" backup
write_legacy "$other" other
write_legacy "$profile" work
# The active library is adopted, not inspected: loading it may migrate it by
# design. Give restore a current-format target so only the backup is under
# test there.
target="$library/target.toml"
cat >"$target" <<'EOF'
[[expansion]]
id = "00000000-0000-4000-8000-000000000001"
trigger = ":target"
replacement = "current format"
EOF
chmod 0600 "$target"

snapshot() {
    find "$library" -print | LC_ALL=C sort
    for file in "$config" "$backup" "$other" "$profile" "$target"; do
        sha256sum "$file"
    done
}
before=$(snapshot)

run() {
    stage=$1
    shift
    if ! "$binary" "$@" >/dev/null 2>"$test_dir/stderr"; then
        printf "error: 'wayexpand %s' failed:\n" "$stage" >&2
        cat "$test_dir/stderr" >&2
        exit 1
    fi
    after=$(snapshot)
    if [ "$after" != "$before" ]; then
        printf "error: 'wayexpand %s' modified the files it inspected\n" "$stage" >&2
        diff -u /dev/fd/3 /dev/fd/4 3<<EOF 4<<EOF2 >&2 || true
$before
EOF
$after
EOF2
        exit 1
    fi
}

run 'config validate' config validate "$other"
run 'config show' config show "$other"
run 'config diff' config diff "$backup" "$other"
run 'check --all' check --all --json "$config"
run 'restore --check' restore "$backup" --check --json "$target"
run 'profile list' profile list "$config"

printf '%s\n' 'read-only configuration commands test passed'
