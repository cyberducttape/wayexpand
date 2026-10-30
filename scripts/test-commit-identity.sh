#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_repo=$(mktemp -d)
# Throwaway signing keys stand in for GitHub's web-flow key: one pinned as
# the trusted key, one that must never be accepted.
GNUPGHOME=$(mktemp -d)
export GNUPGHOME
chmod 0700 "$GNUPGHOME"
cleanup() {
    gpgconf --kill gpg-agent >/dev/null 2>&1 || :
    rm -rf "$temporary_repo" "$GNUPGHOME"
}
trap cleanup EXIT

generate_key() {
    gpg --batch --quiet --passphrase '' --quick-gen-key "$1" ed25519 sign never 2>/dev/null
    gpg --batch --with-colons --list-keys "$1" 2>/dev/null \
        | awk -F: '$1 == "fpr" { print $10; exit }'
}
web_flow_fingerprint=$(generate_key 'Fixture web-flow <web-flow@example.invalid>')
other_fingerprint=$(generate_key 'Fixture other <other@example.invalid>')
web_flow_key="$GNUPGHOME/web-flow.asc"
gpg --batch --armor --export "$web_flow_fingerprint" >"$web_flow_key"

git -C "$temporary_repo" init -q
git -C "$temporary_repo" config user.name 'Stephan Loesevitz'
git -C "$temporary_repo" config user.email 'stephan.loesevitz@gmail.com'

dependabot_name='dependabot[bot]'
dependabot_email='49699333+dependabot[bot]@users.noreply.github.com'
owner_noreply_email='129720061+cyberducttape@users.noreply.github.com'

# commit_with_identity <signing key or ""> author-name author-email
#     committer-name committer-email message
commit_with_identity() {
    local key=$1 name=$2 email=$3 committer_name=$4 committer_email=$5 message=$6
    local signing=(--no-gpg-sign)
    if [ -n "$key" ]; then
        signing=("--gpg-sign=$key")
    fi
    printf '%s\n' "$message" >> "$temporary_repo/identities.txt"
    git -C "$temporary_repo" add identities.txt
    GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email \
        GIT_COMMITTER_NAME=$committer_name GIT_COMMITTER_EMAIL=$committer_email \
        git -C "$temporary_repo" commit -q "${signing[@]}" -m "$message"
}

# merge_with_identity <signing key or ""> author-email: merge a side branch
# the way GitHub's web UI records it.
merge_with_identity() {
    local key=$1 email=$2
    local signing=(--no-gpg-sign)
    if [ -n "$key" ]; then
        signing=("--gpg-sign=$key")
    fi
    local base
    base=$(git -C "$temporary_repo" rev-parse HEAD)
    git -C "$temporary_repo" checkout -q -b side
    commit_with_identity '' 'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' \
        'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' "side change for $email"
    git -C "$temporary_repo" checkout -q "$base"
    GIT_AUTHOR_NAME='Stephan Loesevitz' GIT_AUTHOR_EMAIL=$email \
        GIT_COMMITTER_NAME='GitHub' GIT_COMMITTER_EMAIL='noreply@github.com' \
        git -C "$temporary_repo" merge -q --no-ff "${signing[@]}" -m 'Merge pull request' side
    git -C "$temporary_repo" branch -q -D side
}

check_from_repo() {
    (cd "$temporary_repo" && "$project_dir/scripts/check-commit-identity.sh" \
        --web-flow-key "$web_flow_key" --web-flow-fingerprint "$web_flow_fingerprint" "$@")
}

expect_rejected() {
    local description=$1
    shift
    if check_from_repo "$@" >/dev/null 2>&1; then
        printf '%s\n' "commit identity check unexpectedly accepted $description" >&2
        exit 1
    fi
}

commit_with_identity '' \
    'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' \
    'Stephan Loesevitz' 'stephan.loesevitz@gmail.com' 'approved identity'
check_from_repo HEAD^!

# The committed web-flow key must match the fingerprint pinned in the script.
(cd "$temporary_repo" \
    && "$project_dir/scripts/check-commit-identity.sh" --allow-dependabot --allow-web-merges HEAD^!)

commit_with_identity "$web_flow_fingerprint" \
    "$dependabot_name" "$dependabot_email" 'GitHub' 'noreply@github.com' 'Dependabot dependency update'
commit_with_identity "$web_flow_fingerprint" \
    "$dependabot_name" "$dependabot_email" 'GitHub' 'noreply@github.com' 'Dependabot lockfile update'
expect_rejected 'Dependabot without opt-in' HEAD~2..HEAD
check_from_repo --allow-dependabot HEAD~2..HEAD

commit_with_identity '' \
    "$dependabot_name" "$dependabot_email" 'GitHub' 'noreply@github.com' 'unsigned Dependabot identity'
expect_rejected 'an unsigned Dependabot identity' --allow-dependabot HEAD^!

commit_with_identity "$other_fingerprint" \
    "$dependabot_name" "$dependabot_email" 'GitHub' 'noreply@github.com' 'Dependabot signed by another key'
expect_rejected 'Dependabot signed by an unpinned key' --allow-dependabot HEAD^!

commit_with_identity "$web_flow_fingerprint" \
    "$dependabot_name" 'attacker@example.invalid' 'GitHub' 'noreply@github.com' 'spoofed Dependabot identity'
expect_rejected 'a spoofed Dependabot identity' --allow-dependabot HEAD^!

merge_with_identity "$web_flow_fingerprint" "$owner_noreply_email"
expect_rejected 'a web-UI merge without opt-in' --allow-dependabot HEAD^!
check_from_repo --allow-web-merges HEAD^!

merge_with_identity '' "$owner_noreply_email"
expect_rejected 'an unsigned web-UI merge' --allow-web-merges HEAD^!

merge_with_identity "$web_flow_fingerprint" 'someone@example.invalid'
expect_rejected 'a web-UI merge by another author' --allow-web-merges HEAD^!

commit_with_identity "$web_flow_fingerprint" \
    'Stephan Loesevitz' "$owner_noreply_email" 'GitHub' 'noreply@github.com' 'web-UI squash or edit'
expect_rejected 'a single-parent web-UI commit' --allow-web-merges HEAD^!

if (cd "$temporary_repo" && "$project_dir/scripts/check-commit-identity.sh" \
    --allow-dependabot --web-flow-key "$web_flow_key" \
    --web-flow-fingerprint "$other_fingerprint" HEAD^!) >/dev/null 2>&1; then
    printf '%s\n' 'commit identity check unexpectedly trusted a key file that does not match the pin' >&2
    exit 1
fi

expect_rejected 'a missing ref' refs/heads/__wayexpand_missing_commit_identity_ref__

printf '%s\n' 'commit identity policy accepts the owner and pinned-key-signed, opted-in Dependabot commits and web-UI merges only'
