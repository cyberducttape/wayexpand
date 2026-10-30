#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_repo=$(mktemp -d)
trap 'rm -rf "$temporary_repo"' EXIT

git -C "$temporary_repo" init -q
git -C "$temporary_repo" config user.name 'Stephan Loesevitz'
git -C "$temporary_repo" config user.email 'stephan.loesevitz@gmail.com'

commit_with_identity() {
    local name=$1 email=$2 committer_name=$3 committer_email=$4 message=$5
    printf '%s\n' "$message" >> "$temporary_repo/identities.txt"
    git -C "$temporary_repo" add identities.txt
    GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email \
        GIT_COMMITTER_NAME=$committer_name GIT_COMMITTER_EMAIL=$committer_email \
        git -C "$temporary_repo" commit -q -m "$message"
}

check_from_repo() {
    (cd "$temporary_repo" && "$project_dir/scripts/check-commit-identity.sh" "$@")
}

commit_with_identity \
    'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' \
    'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' 'approved identity'
check_from_repo HEAD^!

commit_with_identity \
    'dependabot[bot]' '49699333+dependabot[bot]@users.noreply.github.com' \
    'GitHub' 'noreply@github.com' 'Dependabot dependency update'
commit_with_identity \
    'dependabot[bot]' '49699333+dependabot[bot]@users.noreply.github.com' \
    'GitHub' 'noreply@github.com' 'Dependabot lockfile update'
if check_from_repo HEAD~2..HEAD >/dev/null 2>&1; then
    printf '%s\n' 'commit identity check unexpectedly accepted Dependabot without opt-in' >&2
    exit 1
fi
check_from_repo --allow-dependabot HEAD~2..HEAD

commit_with_identity \
    'dependabot[bot]' 'attacker@example.invalid' \
    'GitHub' 'noreply@github.com' 'spoofed Dependabot identity'
if check_from_repo --allow-dependabot HEAD^! >/dev/null 2>&1; then
    printf '%s\n' 'commit identity check unexpectedly accepted a spoofed Dependabot identity' >&2
    exit 1
fi

if check_from_repo refs/heads/__wayexpand_missing_commit_identity_ref__ >/dev/null 2>&1; then
    printf '%s\n' 'commit identity check unexpectedly accepted a missing ref' >&2
    exit 1
fi

printf '%s\n' 'commit identity policy accepts the owner and exact opted-in Dependabot identity only'
