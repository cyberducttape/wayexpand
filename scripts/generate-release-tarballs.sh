#!/bin/bash
# Generate dual release tarballs: clean (without vendor/) and vendored (with vendor/)
#
# Usage:
#   ./scripts/generate-release-tarballs.sh <version>
#
# Creates:
#   wayexpand-<version>.tar.gz           - Clean source (Cargo.lock only)
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

# 2. Generate vendored tarball (with vendor/)
printf '%s\n' "Creating vendored tarball (with vendor/)..."
mkdir -p "${tmpdir}/wayexpand-${version}-vendored"
cd "${tmpdir}/wayexpand-${version}-vendored"

# Extract clean tarball
tar -xzf "${tmpdir}/wayexpand-${version}.tar.gz"
cd "wayexpand-${version}"

# Generate vendor/ directory
printf '%s\n' "  Generating vendor/ directory..."
cargo vendor vendor/ >/dev/null 2>&1

# Create vendored tarball
cd ..
tar -czf "${tmpdir}/wayexpand-${version}-vendored.tar.gz" "wayexpand-${version}"

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
printf '%s\n' "  $(ls -lh wayexpand-${version}.tar.gz | awk '{print $5, $9}')"
printf '%s\n' "  Checksum: wayexpand-${version}.tar.gz.sha256"
printf '%s\n' ""
printf '%s\n' "Vendored tarball (offline builds, Launchpad builds):"
printf '%s\n' "  $(ls -lh wayexpand-${version}-vendored.tar.gz | awk '{print $5, $9}')"
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
