#!/bin/sh
# Prepare a WayExpand release by syncing version across all sources.
# This script updates Cargo.toml, debian/changelog, and creates a git tag.
#
# Usage:
#   ./scripts/prepare-release.sh 1.2.0
#
# The script will:
#   1. Verify the new version format (X.Y.Z)
#   2. Update Cargo.toml with the new version
#   3. Update debian/changelog with a new entry
#   4. Commit the changes
#   5. Create a git tag
#
# After running, review the commit/tag, then push:
#   git push origin main v1.2.0

set -eu

if [ $# -ne 1 ]; then
    printf '%s\n' "usage: $0 <version>" >&2
    printf '%s\n' "example: $0 1.2.0" >&2
    exit 2
fi

new_version="$1"

# Validate version format (X.Y.Z)
if ! printf '%s' "$new_version" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    printf '%s\n' "error: version must be in format X.Y.Z (got: $new_version)" >&2
    exit 2
fi

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

# Check if tag already exists
if git rev-parse "v$new_version" >/dev/null 2>&1; then
    printf '%s\n' "error: tag v$new_version already exists" >&2
    exit 1
fi

# Check for uncommitted changes
if ! git diff-index --quiet HEAD --; then
    printf '%s\n' "error: working directory has uncommitted changes" >&2
    printf '%s\n' "stage and commit them first" >&2
    exit 1
fi

printf '%s\n' "Preparing release v$new_version..."

# 1. Update Cargo.toml
printf '%s\n' "Updating Cargo.toml..."
sed -i.bak "s/^version = \"[^\"]*\"/version = \"$new_version\"/" Cargo.toml
rm -f Cargo.toml.bak

# 2. Update debian/changelog
printf '%s\n' "Updating debian/changelog..."
release_date=$(LANG=en_US.UTF-8 date '+%a, %d %b %Y %H:%M:%S %z')
(
    printf '%s\n' "wayexpand ($new_version-1) focal; urgency=medium"
    printf '%s\n' ""
    printf '%s\n' "  * Release v$new_version"
    printf '%s\n' ""
    printf '%s\n' " -- Stephan Loesevitz <stephan.loesevitz@gmail.com>  $release_date"
    printf '%s\n' ""
    cat debian/changelog
) > debian/changelog.tmp
mv debian/changelog.tmp debian/changelog

# 3. Commit changes
printf '%s\n' "Committing version updates..."
git add Cargo.toml debian/changelog
git commit -m "release: version $new_version"

# 4. Create tag
printf '%s\n' "Creating git tag v$new_version..."
git tag -a "v$new_version" -m "Release v$new_version"

printf '%s\n' ""
printf '%s\n' "✓ Release v$new_version prepared successfully"
printf '%s\n' ""
printf '%s\n' "Next steps:"
printf '%s\n' "  1. Review the commit: git log -1"
printf '%s\n' "  2. Review the tag: git show v$new_version"
printf '%s\n' "  3. Push to GitHub: git push origin main v$new_version"
printf '%s\n' ""
printf '%s\n' "The release workflow will then:"
printf '%s\n' "  - Verify all versions match"
printf '%s\n' "  - Build release binaries"
printf '%s\n' "  - Create a GitHub release with prebuilt binaries"
