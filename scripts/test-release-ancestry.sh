#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
check="$project_dir/scripts/check-release-ancestry.sh"
repo=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-ancestry.XXXXXXXXXX")
trap 'rm -rf "$repo"' EXIT

git_() {
    git -C "$repo" -c user.name=test -c user.email=test@invalid \
        -c commit.gpgsign=false -c tag.gpgsign=false "$@"
}
expect_pass() {
    (cd "$repo" && "$check" "$1") >/dev/null 2>&1 || {
        printf '%s\n' "ancestry check rejected valid release: $1" >&2
        exit 1
    }
}
expect_fail() {
    if (cd "$repo" && "$check" "$1") >/dev/null 2>&1; then
        printf '%s\n' "ancestry check accepted invalid release: $1" >&2
        exit 1
    fi
}

git_ init -q -b main
git_ commit -q --allow-empty -m one
git_ tag v1.0.0

# Only the release tag exists: there is nothing to compare against.
expect_fail v1.0.0

git_ commit -q --allow-empty -m two
git_ tag v1.0.1
expect_pass v1.0.1
expect_pass HEAD

# A tag cut from a branch that does not contain the previous release must be
# rejected even though it is the newest tag.
git_ checkout -q --orphan unrelated
git_ commit -q --allow-empty -m unrelated
git_ tag v1.0.2
expect_fail v1.0.2

# Re-checking an older release compares it with its own predecessor, not
# with newer tags.
expect_pass v1.0.1

# Once a broken tag exists, later releases on main fail until the lineage
# is repaired.
git_ checkout -q main
git_ commit -q --allow-empty -m three
git_ tag v1.1.0
expect_fail v1.1.0
git_ tag -d v1.0.2 >/dev/null
expect_pass v1.1.0

printf '%s\n' 'release ancestry check compares against the previous release tag'
