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
project_dir=$(pwd -P)
output_dir=$(pwd -P)
timestamp=$(date -u +%s)
source_commit=$(git -C "$project_dir" rev-parse --short=12 HEAD)
release_tag=$(git -C "$project_dir" describe --tags --exact-match --match "v${version}" HEAD 2>/dev/null || true)
release_inputs_status=$(git -C "$project_dir" status --porcelain=v1 --untracked-files=all -- \
    Cargo.toml Cargo.lock crates debian scripts wayexpand.spec .gitignore .gitattributes)
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-release-${version}-${timestamp}.XXXXXXXXXX")

cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT

printf '%s\n' "Generating release tarballs for v$version..."
printf '%s\n' "Temporary directory: $tmpdir"

# 1. Generate clean tarball (without vendor/)
printf '%s\n' ""
printf '%s\n' "Creating clean tarball (without vendor/)..."
git archive --format=tar --mtime='1970-01-01 00:00:00' \
    --prefix="wayexpand-${version}/" \
    "HEAD^{tree}" | gzip -n > "${tmpdir}/wayexpand-${version}.tar.gz"

# Inspect a saved listing rather than piping tar into grep: with pipefail,
# `grep -q` exits at its first match, tar dies of SIGPIPE, and the pipeline
# reports failure exactly when the forbidden entry is present.
clean_listing="${tmpdir}/clean-contents.txt"
tar -tzf "${tmpdir}/wayexpand-${version}.tar.gz" >"$clean_listing"
if grep -Eq "^wayexpand-${version}/(\.cargo/config\.toml|vendor/)" "$clean_listing"; then
    printf '%s\n' "ERROR: clean tarball contains vendored Cargo configuration or vendor/" >&2
    exit 1
fi

# Verify .git is not in archive
if grep -Eq '(^|/)\.git(/|$)' "$clean_listing"; then
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
# Debian packaging metadata is export-ignored from the generic Git archive,
# but the vendored archive is also the input for offline Debian/Launchpad
# builds, so include it explicitly.
cp -a "$project_dir/debian" "wayexpand-${version}/"
cd "wayexpand-${version}"
printf '%s\n' "$source_commit" > debian/wayexpand-build-sha
if [ "$release_tag" = "v${version}" ] && [ -z "$release_inputs_status" ]; then
    printf '%s\n' "$version" > debian/wayexpand-build-version
fi

# Generate vendor/ and the source replacement config
printf '%s\n' "  Generating vendor/ directory..."
mkdir -p .cargo
cargo vendor --locked vendor/ > .cargo/config.toml

# Create vendored tarball
cd ..
tar -czf "${tmpdir}/wayexpand-${version}-vendored.tar.gz" "wayexpand-${version}"

# Verify .git is not in vendored archive
vendored_listing="${tmpdir}/vendored-contents.txt"
tar -tzf "${tmpdir}/wayexpand-${version}-vendored.tar.gz" >"$vendored_listing"
if grep -Eq '(^|/)\.git(/|$)' "$vendored_listing"; then
    printf '%s\n' "ERROR: .git directory found in vendored tarball" >&2
    exit 1
fi
if ! grep -Fxq "wayexpand-${version}/debian/rules" "$vendored_listing"; then
    printf '%s\n' "ERROR: Debian packaging metadata missing from vendored tarball" >&2
    exit 1
fi
if ! grep -Fxq "wayexpand-${version}/.cargo/config.toml" "$vendored_listing"; then
    printf '%s\n' "ERROR: Cargo source replacement config missing from vendored tarball" >&2
    exit 1
fi
if ! grep -Eq "^wayexpand-${version}/vendor/[^/]+/\.cargo-checksum\.json$" "$vendored_listing"; then
    printf '%s\n' "ERROR: vendored crates missing from vendored tarball" >&2
    exit 1
fi
printf '%s\n' "  ✓ Verified .git not in vendored tarball"

# Move tarballs to current directory
printf '%s\n' ""
printf '%s\n' "Moving tarballs to current directory..."
cd "$output_dir"
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
printf '%s\n' "  5. AUR: Use clean tarball (PKGBUILD fetches dependencies); Fedora RPM: use vendored tarball"
