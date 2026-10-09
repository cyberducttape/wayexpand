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

# Both main push forms must remain inside the branch guard. This extracts the
# guarded block and rejects a future unconditional main update.
guarded_block=$(awk '
    /if \[ "\$GITHUB_REF" = "refs\/heads\/main" \]; then/ { inside=1 }
    inside { print }
    inside && /^          fi$/ { exit }
' "$workflow")
printf '%s\n' "$guarded_block" | grep -F -- 'HEAD:refs/heads/main' >/dev/null

main_pushes=$(grep -Fc -- 'HEAD:refs/heads/main' "$workflow")
test "$main_pushes" -eq 2

printf '%s\n' 'Launchpad workflow safety contract passed'
