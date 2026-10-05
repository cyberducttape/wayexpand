#!/usr/bin/env bash
# Build an unsigned Debian source package from the complete vendored archive.
set -euo pipefail

if [ "$#" -ne 1 ]; then
    printf '%s\n' "usage: $0 <version>" >&2
    exit 2
fi

version="$1"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf '%s\n' "error: version must be numeric SemVer (X.Y.Z)" >&2
    exit 2
fi

output_dir=$(pwd -P)
vendored_archive="$output_dir/wayexpand-${version}-vendored.tar.gz"
if [ ! -f "$vendored_archive" ]; then
    printf 'error: %s is missing; generate the vendored release archive first\n' \
        "$vendored_archive" >&2
    exit 1
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-launchpad-source-${version}.XXXXXXXX")
cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

tar --extract --gzip --file="$vendored_archive" --directory="$tmpdir" --no-same-owner
source_dir="$tmpdir/wayexpand-${version}"
if [ ! -f "$source_dir/debian/control" ] || [ ! -f "$source_dir/.cargo/config.toml" ] \
    || [ ! -d "$source_dir/vendor" ]; then
    printf '%s\n' "error: vendored archive lacks Debian metadata or offline Cargo sources" >&2
    exit 1
fi

package_version=$(dpkg-parsechangelog --file "$source_dir/debian/changelog" --show-field Version)
if [[ "$package_version" != "${version}-"* ]]; then
    printf 'error: Debian version %s does not match archive version %s\n' \
        "$package_version" "$version" >&2
    exit 1
fi

# Debian's 3.0 (quilt) source format keeps Debian metadata in a separate
# debian.tar.xz. The original tarball still contains the exact vendored
# source needed by Launchpad's network-isolated build workers.
mv "$source_dir/debian" "$tmpdir/debian"
tar --create --gzip --file="$tmpdir/wayexpand_${version}.orig.tar.gz" \
    --directory="$tmpdir" "wayexpand-${version}"
mv "$tmpdir/debian" "$source_dir/debian"

(
    cd "$source_dir"
    dpkg-buildpackage -S -sa -us -uc -d
)

artifacts=(
    "$tmpdir/wayexpand_${package_version}.dsc"
    "$tmpdir/wayexpand_${package_version}.debian.tar.xz"
    "$tmpdir/wayexpand_${version}.orig.tar.gz"
    "$tmpdir/wayexpand_${package_version}_source.changes"
    "$tmpdir/wayexpand_${package_version}_source.buildinfo"
)
for artifact in "${artifacts[@]}"; do
    if [ ! -f "$artifact" ]; then
        printf 'error: expected source package artifact missing: %s\n' "$artifact" >&2
        exit 1
    fi
    destination="$output_dir/$(basename "$artifact")"
    if [ -e "$destination" ]; then
        printf 'error: refusing to overwrite existing artifact: %s\n' "$destination" >&2
        exit 1
    fi
done

unpacked_dir="$tmpdir/unpacked-source"
dpkg-source --extract "${artifacts[0]}" "$unpacked_dir"
CARGO_NET_OFFLINE=true cargo metadata --locked --format-version 1 \
    --manifest-path "$unpacked_dir/Cargo.toml" >/dev/null

for artifact in "${artifacts[@]}"; do
    mv "$artifact" "$output_dir/"
done

printf 'Unsigned Launchpad source package %s created:\n' "$package_version"
for artifact in "${artifacts[@]}"; do
    printf '  %s\n' "$(basename "$artifact")"
done
