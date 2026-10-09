#!/usr/bin/env bash
# Publish a self-contained, vendored Git branch for Launchpad's recipe builder.
set -euo pipefail

branch=${LAUNCHPAD_BRANCH:-launchpad-vendored}
remote=${LAUNCHPAD_REMOTE:-launchpad}
project_dir=$(pwd -P)
version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' "$project_dir/Cargo.toml" | head -n1)

if [ -z "$version" ]; then
    printf '%s\n' 'error: unable to read the workspace version' >&2
    exit 1
fi

remote_url=$(git -C "$project_dir" remote get-url "$remote")
remote_ref="refs/heads/$branch"
ssh_command=${GIT_SSH_COMMAND:-ssh -F /dev/null}
remote_commit=$(GIT_SSH_COMMAND="$ssh_command" git -C "$project_dir" ls-remote "$remote_url" "$remote_ref" \
    | awk -v ref="$remote_ref" '$2 == ref { print $1; exit }')
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-launchpad-branch.XXXXXXXX")
cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

mkdir -p "$tmpdir/source"
git -C "$project_dir" archive HEAD | tar --extract --directory "$tmpdir/source"
(
    cd "$tmpdir/source"
    mkdir -p .cargo
    cargo vendor --locked vendor/ > .cargo/config.toml
    git init --quiet
    git config user.name 'Stephan Loesevitz'
    git config user.email 'stephan.loesevitz@gmail.com'
    git add -A
    git add -f .cargo/config.toml vendor
    git commit --quiet -m "Publish vendored Launchpad source for ${version}"
    lease="--force-with-lease=${remote_ref}:${remote_commit}"
    GIT_SSH_COMMAND="$ssh_command" \
        git push "$lease" "$remote_url" "HEAD:$remote_ref"
)

printf 'Published %s to %s (%s)\n' "$branch" "$remote" "$remote_url"
