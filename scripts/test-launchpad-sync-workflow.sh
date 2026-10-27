#!/usr/bin/env bash
# Verify the safety invariants of the Launchpad mirror workflow.
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
workflow="$project_dir/.github/workflows/main.yml"

expected_group="group: launchpad-sync-\${{ github.repository }}"
main_guard="if [ \"\$GITHUB_REF\" = \"refs/heads/main\" ]; then"
tag_message="Skipping Launchpad main update for \$GITHUB_REF; syncing tags only."
main_lease="--force-with-lease=\"refs/heads/main:\$launchpad_main\""
probe_timeout="response=\"\$(timeout --signal=TERM --kill-after=10s 45s ssh"
probe_connect_timeout="-o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=3"
grep -F -- 'for attempt in 1 2 3; do' "$workflow" >/dev/null
grep -F -- 'push_status=$?' "$workflow" >/dev/null
grep -F -- "$expected_group" "$workflow" >/dev/null
grep -F -- 'cancel-in-progress: false' "$workflow" >/dev/null
# The job also vendors and publishes the launchpad-vendored build branch.
grep -F -- 'timeout-minutes: 45' "$workflow" >/dev/null
grep -F -- "$main_guard" "$workflow" >/dev/null
grep -F -- "$tag_message" "$workflow" >/dev/null
grep -F -- "$main_lease" "$workflow" >/dev/null
grep -F -- "$probe_timeout" "$workflow" >/dev/null
grep -F -- "$probe_connect_timeout" "$workflow" >/dev/null

# Authorization probing also retries transient Launchpad SSH failures instead
# of aborting before the already-retried push phase can run.
probe_line=$(grep -nF -- '- name: Verify Launchpad SSH authorization' "$workflow" | cut -d: -f1)
push_line=$(grep -nF -- '- name: Push GitHub main to Launchpad' "$workflow" | cut -d: -f1)
test -n "$probe_line" -a -n "$push_line"
sed -n "$probe_line,$((push_line - 1))p" "$workflow" | grep -F -- 'for attempt in 1 2 3; do' >/dev/null
sed -n "$probe_line,$((push_line - 1))p" "$workflow" | grep -F -- "sleep \$((attempt * 10))" >/dev/null
sed -n "$probe_line,$((push_line - 1))p" "$workflow" | grep -F -- 'after three attempts' >/dev/null

# A failed Launchpad ref lookup must be retried too; otherwise a transient
# path-translation timeout exits before the branch and push retry loops.
grep -F -- 'retry_ls_remote() {' "$workflow" >/dev/null
grep -F -- 'retry_ls_remote launchpad refs/heads/main' "$workflow" >/dev/null
grep -F -- 'retry_ls_remote --tags launchpad' "$workflow" >/dev/null
grep -F -- "Launchpad ref query attempt \$attempt failed" "$workflow" >/dev/null
grep -F -- 'timeout --signal=TERM --kill-after=15s 90s' "$workflow" >/dev/null
push_step=$(grep -nF -- '- name: Push GitHub main to Launchpad' "$workflow" | cut -d: -f1)
vendored_step=$(grep -nF -- '- name: Publish the vendored Launchpad build branch' "$workflow" | cut -d: -f1)
test -n "$push_step" -a -n "$vendored_step"
sed -n "$push_step,$((vendored_step - 1))p" "$workflow" | grep -F -- 'for attempt in 1 2 3 4 5; do' >/dev/null
sed -n "$push_step,$((vendored_step - 1))p" "$workflow" | grep -F -- 'if [ "$attempt" -lt 5 ]; then' >/dev/null

# Both main push forms must remain inside the branch guard. The two push lines
# must occur after the guard and before the tag-only branch, so nested `if`
# blocks cannot accidentally move one of them outside the protection.
guard_line=$(grep -nF -- "$main_guard" "$workflow" | cut -d: -f1)
first_push=$(grep -nF -- 'HEAD:refs/heads/main' "$workflow" | sed -n '1p' | cut -d: -f1)
second_push=$(grep -nF -- 'HEAD:refs/heads/main' "$workflow" | sed -n '2p' | cut -d: -f1)
tag_branch=$(grep -nF -- "$tag_message" "$workflow" | cut -d: -f1)
test -n "$guard_line" -a -n "$first_push" -a -n "$second_push" -a -n "$tag_branch"
test "$guard_line" -lt "$first_push"
test "$first_push" -lt "$second_push"
test "$second_push" -lt "$tag_branch"
test "$(grep -Fc -- 'HEAD:refs/heads/main' "$workflow")" -eq 2

# The vendored build branch is published only for main, never from a tag's
# detached HEAD, and each attempt is time-bounded.
publish_step="- name: Publish the vendored Launchpad build branch"
publish_line=$(grep -nF -- "$publish_step" "$workflow" | cut -d: -f1)
test -n "$publish_line"
sed -n "$((publish_line + 1))p" "$workflow" | grep -F -- "if: github.ref == 'refs/heads/main'" >/dev/null
sed -n "$publish_line,\$p" "$workflow" | grep -F -- 'timeout --signal=TERM --kill-after=30s 15m' >/dev/null
sed -n "$publish_line,\$p" "$workflow" | grep -F -- 'bash scripts/publish-launchpad-branch.sh' >/dev/null

printf '%s\n' 'Launchpad workflow safety contract passed'
