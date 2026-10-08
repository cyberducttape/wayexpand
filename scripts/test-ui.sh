#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_config=$(mktemp "$project_dir/.wayexpand-ui-test.XXXXXX")
output=$(mktemp "$project_dir/.wayexpand-ui-output.XXXXXX")
lock_file="$project_dir/.$(basename "$test_config").wayexpand.lock"
trap 'rm -f "$test_config" "$output" "$lock_file"' EXIT INT TERM

cp "$project_dir/expansions.toml" "$test_config"
chmod 0600 "$test_config"

command -v script >/dev/null 2>&1
command -v timeout >/dev/null 2>&1

# Keep this smoke test focused on terminal startup and clean shutdown. CRUD
# behavior is covered by the UI unit tests; replaying a long stream of timing-
# sensitive keystrokes through `script` is not a reliable integration test
# across terminal implementations.
(sleep 0.5; printf '%s' 'q') |
    timeout 15 script -qec "$project_dir/target/debug/wayexpand-ui $test_config" /dev/null >"$output" 2>&1

grep -F 'WayExpand Settings' "$output" >/dev/null
grep -F 'Ready' "$output" >/dev/null
printf '%s\n' 'UI startup/shutdown smoke test passed'
