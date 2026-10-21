#!/usr/bin/env bash
set -euo pipefail

release_ref=${1:-HEAD}
semver_pattern='^v[0-9]+\.[0-9]+\.[0-9]+$'

release_commit=$(git rev-parse --verify "$release_ref^{commit}")

# Compare against the newest release tag *before* this one. The tag being
# released is itself the newest tag, so picking the top of the list would
# compare the release with itself and always pass.
previous_tag=
while IFS= read -r tag; do
    [ -n "$tag" ] || continue
    if [[ "$release_ref" =~ $semver_pattern ]]; then
        [ "$tag" != "$release_ref" ] || continue
        lowest=$(printf '%s\n%s\n' "$tag" "$release_ref" | sort -V | head -n1)
        [ "$lowest" = "$tag" ] || continue
    else
        [ "$(git rev-parse "$tag^{commit}")" != "$release_commit" ] || continue
    fi
    previous_tag=$tag
    break
done < <(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-version:refname)

if [ -z "$previous_tag" ]; then
    echo "no earlier semver release tag exists for $release_ref; cannot verify release ancestry" >&2
    echo "(a shallow checkout without tags also causes this; fetch full history)" >&2
    exit 1
fi

git rev-parse --verify "$previous_tag^{commit}" >/dev/null

if ! git merge-base --is-ancestor "$previous_tag" "$release_commit"; then
    echo "release provenance check failed: $release_ref is not descended from $previous_tag" >&2
    echo "repair the release lineage or create an explicitly reviewed migration tag before publishing" >&2
    exit 1
fi

printf 'release ancestry passed: %s descends from %s\n' "$release_ref" "$previous_tag"
