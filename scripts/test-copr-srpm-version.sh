#!/bin/sh
# Check how scripts/copr-srpm.sh names Copr builds: a checkout at the
# release tag is X.Y.Z-1, later commits get a unique, increasing
# development release, and a checkout without the tag is refused. RPM
# ordering is checked with rpmdev-vercmp when it is installed.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
repo="$work/repo"
git init -q "$repo"
cd "$repo"
git config user.name test
git config user.email test@example.invalid
mkdir scripts
cp "$project_dir/scripts/copr-srpm.sh" scripts/
printf 'Name: wayexpand\nVersion:        1.4.0\nRelease:        1%%{?dist}\n' >wayexpand.spec
git add . && git commit -q -m one
release() { sh scripts/copr-srpm.sh --print-release; }

# No tag for the spec's version: refused.
if release >/dev/null 2>&1; then
    printf '%s\n' "FAIL: built without the release tag" >&2
    exit 1
fi

git tag 1.4.0
stable=$(release)
[ "$stable" = "1.4.0-1" ] || {
    printf '%s\n' "FAIL: stable build is $stable" >&2
    exit 1
}

git commit -q --allow-empty -m two
first=$(release)
git commit -q --allow-empty -m three
second=$(release)
case "$first" in
1.4.0-1.1.git*) ;;
*)
    printf '%s\n' "FAIL: first development build is $first" >&2
    exit 1
    ;;
esac
case "$second" in
1.4.0-1.2.git*) ;;
*)
    printf '%s\n' "FAIL: second development build is $second" >&2
    exit 1
    ;;
esac
[ "$first" != "$second" ] || {
    printf '%s\n' "FAIL: two commits share $first" >&2
    exit 1
}

# A vX.Y.Z tag works too.
git tag -d 1.4.0 >/dev/null
git tag v1.4.0 HEAD~2
[ "$(release)" = "$second" ] || {
    printf '%s\n' "FAIL: v-prefixed tag changed the release" >&2
    exit 1
}

if command -v rpmdev-vercmp >/dev/null 2>&1; then
    newer() {
        rpmdev-vercmp "$1" "$2" >/dev/null 2>&1
        [ $? -eq 11 ]
    }
    newer "$first" "$stable" || {
        printf '%s\n' "FAIL: $first does not upgrade $stable" >&2
        exit 1
    }
    newer "$second" "$first" || {
        printf '%s\n' "FAIL: $second does not upgrade $first" >&2
        exit 1
    }
    newer "1.4.1-1" "$second" || {
        printf '%s\n' "FAIL: the next release does not upgrade $second" >&2
        exit 1
    }
fi

printf '%s\n' "Copr SRPM versioning passed"
