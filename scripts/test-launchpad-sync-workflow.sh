#!/usr/bin/env bash
# Verify the safety invariants of the Launchpad mirror workflow.
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow="$project_dir/.github/workflows/main.yml"

grep -F -- 'group: launchpad-sync-${{ github.repository }}' "$workflow" >/dev/null
grep -F -- 'cancel-in-progress: false' "$workflow" >/dev/null
grep -F -- 'if [ "$GITHUB_REF" = "refs/heads/main" ]; then' "$workflow" >/dev/null
grep -F -- 'Skipping Launchpad main update for $GITHUB_REF; syncing tags only.' "$workflow" >/dev/null
grep -F -- '--force-with-lease="refs/heads/main:$launchpad_main"' "$workflow" >/dev/null

# Both main push forms must remain inside the branch guard. The two push lines
# must occur after the guard and before the tag-only branch, so nested `if`
# blocks cannot accidentally move one of them outside the protection.
guard_line=$(grep -nF -- 'if [ "$GITHUB_REF" = "refs/heads/main" ]; then' "$workflow" | cut -d: -f1)
first_push=$(grep -nF -- 'HEAD:refs/heads/main' "$workflow" | sed -n '1p' | cut -d: -f1)
second_push=$(grep -nF -- 'HEAD:refs/heads/main' "$workflow" | sed -n '2p' | cut -d: -f1)
tag_branch=$(grep -nF -- 'Skipping Launchpad main update for $GITHUB_REF; syncing tags only.' "$workflow" | cut -d: -f1)
test -n "$guard_line" -a -n "$first_push" -a -n "$second_push" -a -n "$tag_branch"
test "$guard_line" -lt "$first_push"
test "$first_push" -lt "$second_push"
test "$second_push" -lt "$tag_branch"
test "$(grep -Fc -- 'HEAD:refs/heads/main' "$workflow")" -eq 2

printf '%s\n' 'Launchpad workflow safety contract passed'
