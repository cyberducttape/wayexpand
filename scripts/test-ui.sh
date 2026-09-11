#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_config=$(mktemp "${TMPDIR:-/tmp}/wayexpand-ui-test.XXXXXX")
output=$(mktemp "${TMPDIR:-/tmp}/wayexpand-ui-output.XXXXXX")
trap 'rm -f "$test_config" "$output"' EXIT INT TERM

cp "$project_dir/expansions.toml" "$test_config"
chmod 0600 "$test_config"

command -v script >/dev/null 2>&1
command -v timeout >/dev/null 2>&1

(
    sleep 0.2
    printf '%s' 'n'
    sleep 0.1
    printf '%s' ':ui-test'
    sleep 0.1
    printf '\r'
    sleep 0.1
    printf '%s' 'Created by the UI test'
    sleep 0.1
    printf '\r'
    sleep 0.1
    printf '%s' 'u'
    sleep 0.5
    printf '%s' 'm'
    sleep 0.5
    printf '%s' 'D'
    sleep 0.2
    printf '\025'
    printf '%s' 'Updated by UI'
    sleep 0.2
    printf '\r'
    sleep 0.3
    printf '%s' 't'
    sleep 0.2
    printf '\025'
    printf '%s' 'edited, ui'
    sleep 0.2
    printf '\r'
    sleep 0.5
    printf '%s' 'q'
) | timeout 5 script -qec "$project_dir/target/debug/wayexpand-ui $test_config" /dev/null >"$output" 2>&1

grep -F 'Snippet created' "$output" >/dev/null
grep -F 'Undid the last saved change' "$output" >/dev/null
grep -F 'Word-boundary matching enabled' "$output" >/dev/null
grep -F 'Updated description for' "$output" >/dev/null
grep -F 'Updated tags for' "$output" >/dev/null
grep -F 'match_mode = "word-boundary"' "$test_config" >/dev/null
grep -F 'description = "Updated by UI"' "$test_config" >/dev/null
grep -F 'edited' "$test_config" >/dev/null
if grep -F 'trigger = ":ui-test"' "$test_config" >/dev/null; then
    printf '%s\n' "UI undo test did not remove the last edit" >&2
    exit 1
fi

(sleep 0.2; printf '%s' 'E'; sleep 0.2; printf '%s' 'q') |
    VISUAL=/usr/bin/true timeout 5 script -qec "$project_dir/target/debug/wayexpand-ui $test_config" /dev/null >"$output" 2>&1
grep -F 'Replacement edited in external editor' "$output" >/dev/null
printf '%s\n' "UI CRUD test passed"
