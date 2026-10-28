#!/bin/bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
    printf '%s\n' "usage: $0 <version>" >&2
    exit 2
fi
version=$1

source_commit=$(git rev-parse --short=12 HEAD)
build_sha=${WAYEXPAND_BUILD_SHA:-$source_commit}
if [[ ! "$build_sha" =~ ^[0-9a-f]{12,40}$ ]]; then
    printf '%s\n' 'error: package build needs a 12- to 40-character lowercase hex source commit' >&2
    exit 1
fi
if [ "$build_sha" != "$source_commit" ]; then
    printf '%s\n' 'error: WAYEXPAND_BUILD_SHA does not match the checked-out source revision' >&2
    exit 1
fi
release_tag=$(git describe --tags --exact-match --match "v${version}" HEAD 2>/dev/null || true)
release_inputs_status=$(git status --porcelain=v1 --untracked-files=all -- \
    Cargo.toml Cargo.lock crates debian scripts wayexpand.spec .gitignore .gitattributes)
build_release_version=${WAYEXPAND_BUILD_RELEASE_VERSION:-}
if [ -n "$build_release_version" ] && [ "$build_release_version" != "$version" ]; then
    printf '%s\n' 'error: package release identity does not match the requested package version' >&2
    exit 1
fi
if [ -z "$build_release_version" ] && [ -z "${WAYEXPAND_BUILD_SHA:-}" ] \
    && [ "$release_tag" = "v${version}" ] && [ -z "$release_inputs_status" ]; then
    build_release_version=$version
fi
if [ -n "$build_release_version" ] \
    && { [ "$release_tag" != "v${version}" ] || [ -n "$release_inputs_status" ]; }; then
    printf '%s\n' 'error: exact release identity requires a clean source tree at its matching release tag' >&2
    exit 1
fi
export WAYEXPAND_BUILD_SHA=$build_sha
if [ -n "$build_release_version" ]; then
    export WAYEXPAND_BUILD_RELEASE_VERSION=$build_release_version
else
    unset WAYEXPAND_BUILD_RELEASE_VERSION || true
fi

if [ ! -f .cargo/config.toml ] || [ ! -d vendor ]; then
    printf '%s\n' 'release package builds require cargo vendor output (.cargo/config.toml and vendor/)' >&2
    exit 1
fi

dpkg-buildpackage -us -uc -b -d
deb="../wayexpand_${version}-1_amd64.deb"
if [ ! -f "$deb" ]; then
    printf '%s\n' 'Debian package build produced no amd64 package' >&2
    exit 1
fi
install -m 0644 "$deb" .
sha256sum "$(basename "$deb")" > "$(basename "$deb").sha256"

top=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-rpm-${version}.XXXXXXXX")
trap 'rm -rf "$top"' EXIT INT TERM
source_dir="$top/sources/wayexpand-${version}"
mkdir -p "$source_dir" "$top/rpmbuild/SOURCES"
git archive --format=tar "HEAD^{tree}" | tar -xf - -C "$source_dir"
cp -a .cargo vendor "$source_dir/"
mkdir -p "$source_dir/debian"
printf '%s\n' "$build_sha" > "$source_dir/debian/wayexpand-build-sha"
if [ -n "$build_release_version" ]; then
    printf '%s\n' "$build_release_version" > "$source_dir/debian/wayexpand-build-version"
fi
tar -C "$top/sources" -czf "$top/rpmbuild/SOURCES/wayexpand-${version}-vendored.tar.gz" \
    "wayexpand-${version}"
rpmbuild -bb --nodeps \
    --define "_topdir $top/rpmbuild" \
    --define "_sourcedir $top/rpmbuild/SOURCES" \
    wayexpand.spec
rpm=$(find "$top/rpmbuild/RPMS" -type f -name 'wayexpand-*.rpm' -print -quit)
if [ -z "$rpm" ]; then
    printf '%s\n' 'RPM package build produced no package' >&2
    exit 1
fi
install -m 0644 "$rpm" .
sha256sum "$(basename "$rpm")" > "$(basename "$rpm").sha256"
