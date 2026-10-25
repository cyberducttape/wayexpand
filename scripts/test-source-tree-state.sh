#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-source-tree-state.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
git -C "$test_root" init -q
git -C "$test_root" config user.name 'WayExpand test'
git -C "$test_root" config user.email 'wayexpand-test@example.invalid'
mkdir "$test_root/crates"
printf 'tracked input\n' >"$test_root/crates/input.txt"
git -C "$test_root" add crates/input.txt
git -C "$test_root" commit -qm 'initial'

state=$(sh "$project_dir/scripts/source-tree-state.sh" "$test_root" crates)
[ "$state" = clean ]
printf 'generated report\n' >"$test_root/report.json"
state=$(sh "$project_dir/scripts/source-tree-state.sh" "$test_root" crates)
[ "$state" = clean ]
printf 'new source input\n' >"$test_root/crates/new.txt"
state=$(sh "$project_dir/scripts/source-tree-state.sh" "$test_root" crates)
[ "$state" = dirty ]
printf '%s\n' 'source tree state path filtering passed'
