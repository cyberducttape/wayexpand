#!/usr/bin/env bash
# Publish a self-contained, vendored Git branch for Launchpad's recipe builder.
#
# The recipe builds this branch as is, so it must carry both Cargo's vendored
# sources and the debian/ packaging that `git archive` leaves out
# (.gitattributes marks debian/ export-ignore for release tarballs).
#
# With --output DIR the tree is written to DIR and nothing is pushed.
set -euo pipefail

output_dir=
if [ "${1:-}" = --output ]; then
    output_dir=${2:?--output needs a directory}
    shift 2
fi
if [ "$#" -ne 0 ]; then
    printf '%s\n' 'usage: publish-launchpad-branch.sh [--output DIR]' >&2
    exit 2
fi

branch=${LAUNCHPAD_BRANCH:-launchpad-vendored}
remote=${LAUNCHPAD_REMOTE:-launchpad}
project_dir=$(pwd -P)
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' "$project_dir/Cargo.toml" | head -n1)

if [ -z "$version" ]; then
    printf '%s\n' 'error: unable to read the workspace version' >&2
    exit 1
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-launchpad-branch.XXXXXXXX")
cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

source_dir=${output_dir:-$tmpdir/source}
if [ -n "$output_dir" ] && [ -e "$output_dir" ]; then
    printf 'error: %s already exists\n' "$output_dir" >&2
    exit 1
fi
mkdir -p "$source_dir"
git -C "$project_dir" archive HEAD | tar --extract --directory "$source_dir"
# git archive honours export-ignore even for a named path, so check debian/
# out of HEAD through a private index instead.
GIT_INDEX_FILE="$tmpdir/index" git -C "$project_dir" read-tree HEAD
git -C "$project_dir" ls-tree -r -z --name-only HEAD -- debian \
    | GIT_INDEX_FILE="$tmpdir/index" git -C "$project_dir" checkout-index \
        -z --stdin --prefix="$source_dir/"
sed -i '/^debian\/ export-ignore$/d' "$source_dir/.gitattributes"
for required in debian/control debian/rules debian/changelog debian/source/format; do
    if [ ! -f "$source_dir/$required" ]; then
        printf 'error: the build branch is missing %s\n' "$required" >&2
        exit 1
    fi
done
(
    cd "$source_dir"
    mkdir -p .cargo
    cargo vendor --locked vendor/ > .cargo/config.toml
)
if [ -n "$output_dir" ]; then
    printf 'Wrote the Launchpad build tree to %s\n' "$output_dir"
    exit 0
fi

remote_url=$(git -C "$project_dir" remote get-url "$remote")
remote_ref="refs/heads/$branch"
ssh_command=${GIT_SSH_COMMAND:-ssh -F /dev/null}
remote_commit=$(GIT_SSH_COMMAND="$ssh_command" git -C "$project_dir" ls-remote "$remote_url" "$remote_ref" \
    | awk -v ref="$remote_ref" '$2 == ref { print $1; exit }')
(
    cd "$source_dir"
    git init --quiet
    git config user.name 'Stephan Loesevitz'
    git config user.email 'stephan.loesevitz@gmail.com'
    git add -A
    git add -f .cargo/config.toml vendor
    git commit --quiet -m "Publish vendored Launchpad source for ${version}"
    if [ -n "$remote_commit" ]; then
        # Fetch the published tip so the push sends only changed objects
        # rather than the whole vendor tree, and skip an unchanged tree.
        GIT_SSH_COMMAND="$ssh_command" \
            git fetch --quiet --depth=1 "$remote_url" "$remote_ref"
        if [ "$(git rev-parse 'FETCH_HEAD^{tree}')" = "$(git rev-parse 'HEAD^{tree}')" ]; then
            printf '%s already matches %s; nothing to publish\n' "$branch" "$(git -C "$project_dir" rev-parse --short HEAD)"
            exit 0
        fi
    fi
    lease="--force-with-lease=${remote_ref}:${remote_commit}"
    GIT_SSH_COMMAND="$ssh_command" \
        git push "$lease" "$remote_url" "HEAD:$remote_ref"
)

printf 'Published %s to %s (%s)\n' "$branch" "$remote" "$remote_url"
