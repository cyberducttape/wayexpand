#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
missing_ref='refs/heads/__wayexpand_missing_commit_identity_ref__'

if "$project_dir/scripts/check-commit-identity.sh" "$missing_ref" >/dev/null 2>&1; then
    printf '%s\n' "commit identity check unexpectedly accepted invalid ref: $missing_ref" >&2
    exit 1
fi

printf '%s\n' 'commit identity script rejects invalid refs'
