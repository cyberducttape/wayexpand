#!/usr/bin/env bash
set -euo pipefail

release_ref=${1:-HEAD}
latest_tag=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-version:refname | head -n1)

if [ -z "$latest_tag" ]; then
    echo "no semver release tag exists; cannot verify release ancestry" >&2
    exit 1
fi

git rev-parse --verify "$release_ref^{commit}" >/dev/null
git rev-parse --verify "$latest_tag^{commit}" >/dev/null

if ! git merge-base --is-ancestor "$latest_tag" "$release_ref"; then
    echo "release provenance check failed: $release_ref is not descended from $latest_tag" >&2
    echo "repair the release lineage or create an explicitly reviewed migration tag before publishing" >&2
    exit 1
fi

printf 'release ancestry passed: %s descends from %s\n' "$release_ref" "$latest_tag"
