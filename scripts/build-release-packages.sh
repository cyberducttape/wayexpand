#!/bin/bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
    printf '%s\n' "usage: $0 <version>" >&2
    exit 2
fi
version=$1

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
git archive --format=tar HEAD^{tree} | tar -xf - -C "$source_dir"
cp -a .cargo vendor "$source_dir/"
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
