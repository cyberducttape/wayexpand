#!/bin/sh
# Build the WayExpand source RPM for Fedora Copr from this checkout.
#
#   scripts/copr-srpm.sh OUTDIR [SPEC]
#   scripts/copr-srpm.sh --print-release [SPEC]   # only print VERSION-RELEASE
#
# A checkout exactly at the release tag of the spec's Version (X.Y.Z or
# vX.Y.Z) is a stable build: X.Y.Z-1. Any later commit is a development
# build, X.Y.Z-1.N.gitSHA, where N counts the commits since the tag. That
# sorts after the release and before the next one, grows along main, and
# names the exact source commit, so two different builds never share an RPM
# version. A checkout without the tag, or not descending from it, is refused
# rather than published under a version it does not match.
set -eu

print_only=false
if [ "${1:-}" = "--print-release" ]; then
    print_only=true
    shift
    spec=${1:-wayexpand.spec}
else
    [ $# -ge 1 ] || {
        printf '%s\n' "usage: $0 OUTDIR [SPEC] | --print-release [SPEC]" >&2
        exit 2
    }
    outdir=$1
    spec=${2:-wayexpand.spec}
fi

top=$(git rev-parse --show-toplevel)
cd "$top"
version=$(sed -n 's/^Version:[[:space:]]*//p' "$spec")
[ -n "$version" ] || {
    printf '%s\n' "error: $spec has no Version" >&2
    exit 1
}

tag=
for candidate in "$version" "v$version"; do
    if git rev-parse -q --verify "refs/tags/$candidate^{commit}" >/dev/null; then
        tag=$candidate
        break
    fi
done
[ -n "$tag" ] || {
    printf '%s\n' "error: no release tag $version or v$version in this checkout; Copr builds need the tag of the spec's Version (fetch tags, or prepare that release first)" >&2
    exit 1
}
tagged=$(git rev-parse "refs/tags/$tag^{commit}")
if [ "$tagged" = "$(git rev-parse HEAD)" ]; then
    kind=stable
    release=1
else
    git merge-base --is-ancestor "$tagged" HEAD || {
        printf '%s\n' "error: HEAD does not descend from release tag $tag" >&2
        exit 1
    }
    kind=development
    release="1.$(git rev-list --count "$tagged..HEAD").git$(git rev-parse --short=12 HEAD)"
fi

if $print_only; then
    printf '%s-%s\n' "$version" "$release"
    exit 0
fi
printf 'Copr %s build: %s-%s\n' "$kind" "$version" "$release"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
bash scripts/generate-release-tarballs.sh "$version"
mkdir -p "$outdir"
mv "wayexpand-$version-vendored.tar.gz" "$work/"
rm -f "wayexpand-$version.tar.gz" "wayexpand-$version"*.sha256
sed "s/^Release:.*/Release:        $release%{?dist}/" "$spec" >"$work/wayexpand.spec"
grep -qx "Release:        $release%{?dist}" "$work/wayexpand.spec"
rpmbuild -bs \
    --define "_sourcedir $work" \
    --define "_srcrpmdir $outdir" \
    "$work/wayexpand.spec"
