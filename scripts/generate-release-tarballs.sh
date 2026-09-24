#!/bin/bash
# Generate dual release tarballs: clean (without vendor/) and vendored (with vendor/)
#
# Usage:
#   ./scripts/generate-release-tarballs.sh <version>
#
# Creates:
#   wayexpand-<version>.tar.gz           - Clean source (Cargo.lock, no vendor config)
#   wayexpand-<version>-vendored.tar.gz  - With vendor/ (offline builds)
#   SHA256 checksums for both

set -euo pipefail

if [ $# -ne 1 ]; then
    printf '%s\n' "usage: $0 <version>" >&2
    printf '%s\n' "example: $0 1.2.0" >&2
    exit 2
fi

version="$1"
timestamp=$(date -u +%s)
tmpdir=$(mktemp -d "/tmp/wayexpand-release-${version}-${timestamp}.XXXXXXXXXX")

cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

printf '%s\n' "Generating release tarballs for v$version..."
printf '%s\n' "Temporary directory: $tmpdir"

# 1. Generate clean tarball (without vendor/)
printf '%s\n' ""
printf '%s\n' "Creating clean tarball (without vendor/)..."
git archive --format=tar.gz \
    --prefix="wayexpand-${version}/" \
    --output="${tmpdir}/wayexpand-${version}.tar.gz" \
    HEAD

if tar -tzf "${tmpdir}/wayexpand-${version}.tar.gz" \
    | grep -E "^wayexpand-${version}/(\.cargo/config\.toml|vendor/)" >/dev/null; then
    printf '%s\n' "ERROR: clean tarball contains vendored Cargo configuration or vendor/" >&2
    exit 1
fi

# Verify .git is not in archive
if tar -tzf "${tmpdir}/wayexpand-${version}.tar.gz" | grep -q '\.git/'; then
    printf '%s\n' "ERROR: .git directory found in clean tarball" >&2
    exit 1
fi
printf '%s\n' "  ✓ Verified .git not in clean tarball"

# 2. Generate vendored tarball (with vendor/)
printf '%s\n' "Creating vendored tarball (with vendor/)..."
mkdir -p "${tmpdir}/wayexpand-${version}-vendored"
cd "${tmpdir}/wayexpand-${version}-vendored"

# Extract clean tarball
tar -xzf "${tmpdir}/wayexpand-${version}.tar.gz"
cd "wayexpand-${version}"

# Generate vendor/ and the source replacement config
printf '%s\n' "  Generating vendor/ directory..."
mkdir -p .cargo
cargo vendor vendor/ > .cargo/config.toml

# Create vendored tarball
cd ..
tar -czf "${tmpdir}/wayexpand-${version}-vendored.tar.gz" "wayexpand-${version}"

# Verify .git is not in vendored archive
if tar -tzf "${tmpdir}/wayexpand-${version}-vendored.tar.gz" | grep -q '\.git/'; then
    printf '%s\n' "ERROR: .git directory found in vendored tarball" >&2
    exit 1
fi
printf '%s\n' "  ✓ Verified .git not in vendored tarball"

# Move tarballs to current directory
printf '%s\n' ""
printf '%s\n' "Moving tarballs to current directory..."
mv "${tmpdir}/wayexpand-${version}.tar.gz" .
mv "${tmpdir}/wayexpand-${version}-vendored.tar.gz" .

# Generate SHA256 checksums
printf '%s\n' "Generating checksums..."
sha256sum "wayexpand-${version}.tar.gz" > "wayexpand-${version}.tar.gz.sha256"
sha256sum "wayexpand-${version}-vendored.tar.gz" > "wayexpand-${version}-vendored.tar.gz.sha256"

# Summary
printf '%s\n' ""
printf '%s\n' "✓ Release tarballs generated successfully"
printf '%s\n' ""
printf '%s\n' "Clean tarball (recommended for distributions):"
printf '%s\n' "  $(du -h -- "wayexpand-${version}.tar.gz" | awk '{print $1, $2}')"
printf '%s\n' "  Checksum: wayexpand-${version}.tar.gz.sha256"
printf '%s\n' ""
printf '%s\n' "Vendored tarball (offline builds, Launchpad builds):"
printf '%s\n' "  $(du -h -- "wayexpand-${version}-vendored.tar.gz" | awk '{print $1, $2}')"
printf '%s\n' "  Checksum: wayexpand-${version}-vendored.tar.gz.sha256"
printf '%s\n' ""
printf '%s\n' "Checksums:"
cat "wayexpand-${version}.tar.gz.sha256"
cat "wayexpand-${version}-vendored.tar.gz.sha256"
printf '%s\n' ""
printf '%s\n' "Publishing:"
printf '%s\n' "  1. Create GitHub release for v$version"
printf '%s\n' "  2. Upload wayexpand-${version}.tar.gz (+ .sha256)"
printf '%s\n' "  3. Upload wayexpand-${version}-vendored.tar.gz (+ .sha256)"
printf '%s\n' "  4. Launchpad PPA: Use vendored tarball in debian/changelog"
printf '%s\n' "  5. AUR/Copr: Use clean tarball (cargo vendor is in build() function)"
