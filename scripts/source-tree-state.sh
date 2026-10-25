#!/bin/sh
# Report whether build and harness inputs in a checkout have local changes.
# Usage: source-tree-state.sh REPOSITORY [PATHSPEC ...]
set -eu

repository=${1:?repository path required}
shift
if ! git -C "$repository" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    printf '%s\n' unknown
    exit 0
fi
if [ "$#" -eq 0 ]; then
    printf '%s\n' 'error: at least one input pathspec is required' >&2
    exit 2
fi
if [ -z "$(git -C "$repository" status --porcelain --untracked-files=all -- "$@")" ]; then
    printf '%s\n' clean
else
    printf '%s\n' dirty
fi
