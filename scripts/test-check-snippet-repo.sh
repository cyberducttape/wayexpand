#!/bin/sh
# Tests for scripts/check-snippet-repo.sh against a throwaway repository.
#
#   test-check-snippet-repo.sh [WAYEXPAND_CLI]
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
checker="$project_dir/scripts/check-snippet-repo.sh"
WAYEXPAND_CLI=${1:-${WAYEXPAND_CLI:-$project_dir/target/debug/wayexpand}}
export WAYEXPAND_CLI
repo=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-check-test.XXXXXX")
trap 'rm -rf "$repo"' EXIT INT TERM
cd "$repo"
umask 022

mkdir -p team packs
cp -R "$project_dir/packs/kubernetes" packs/
cat >team/support.toml <<'TOML'
[[expansion]]
trigger = ";ty"
replacement = "Thank you for your patience."
TOML
cat >Cargo.toml <<'TOML'
[package]
name = "not-snippets"
TOML

run() {
    status=0
    sh "$checker" "$@" >"$repo/.out" 2>&1 || status=$?
}
expect_status() {
    [ "$status" -eq "$1" ] || {
        printf 'expected exit %s, got %s:\n' "$1" "$status" >&2
        cat "$repo/.out" >&2
        exit 1
    }
}
expect_output() {
    grep -F -- "$1" "$repo/.out" >/dev/null || {
        printf 'missing %s in:\n' "$1" >&2
        cat "$repo/.out" >&2
        exit 1
    }
}

# World-readable checkouts pass; non-snippet TOML is skipped.
run
expect_status 0
expect_output 'ok   team/support.toml (1 expansions, 0 commands)'
expect_output 'ok   packs/kubernetes'
if grep -F Cargo.toml "$repo/.out" >/dev/null; then
    printf '%s\n' 'non-snippet TOML was checked' >&2
    exit 1
fi

# Risks warn by default and fail with --deny-risks, with annotations.
cat >team/risky.toml <<'TOML'
[[expansion]]
trigger = "sig"
replacement = "Best regards"
TOML
run team/risky.toml
expect_status 0
expect_output 'WARN team/risky.toml: trigger sig ordinary letters'
GITHUB_ACTIONS=true run --deny-risks team/risky.toml Cargo.toml
expect_status 1
expect_output '::warning file=team/risky.toml::trigger sig'
expect_output '::error file=team/risky.toml::1 trigger risk(s) found'

# Invalid configuration fails without leaking the replacement.
cat >team/broken.toml <<'TOML'
[[expansion]]
trigger = ";x"
replacement = "secret-replacement-text"
unknown_field = true
TOML
run team
expect_status 1
expect_output 'FAIL team/broken.toml'
if grep -F secret-replacement-text "$repo/.out" >/dev/null; then
    printf '%s\n' 'checker output leaked snippet content' >&2
    exit 1
fi
rm team/broken.toml team/risky.toml

# An unsigned pack fails signature verification.
: >"$repo/signers"
run --signers "$repo/signers" packs/kubernetes
expect_status 1
expect_output 'FAIL packs/kubernetes: pack signature'

# Paths outside the repository and names with whitespace are refused.
run /etc/hostname
expect_status 1
expect_output 'not inside the current directory'
cp team/support.toml 'team/with space.toml'
run team
expect_status 1
expect_output 'whitespace'

printf '%s\n' 'snippet repository checker tests passed'
