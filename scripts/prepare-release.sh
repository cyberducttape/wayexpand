#!/bin/sh
# Prepare a WayExpand release by syncing version across all sources.
# This script updates Cargo.toml/Cargo.lock, CHANGELOG.md, debian/changelog,
# distro and desktop metadata, and creates a git tag.
#
# Usage:
#   ./scripts/prepare-release.sh 1.2.0
#
# The script will:
#   1. Verify the new version format (X.Y.Z)
#   2. Update Cargo.toml/Cargo.lock, CHANGELOG.md, distro and desktop metadata
#   3. Update debian/changelog with a new entry
#   4. Commit the changes as Stephan Loesevitz
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

previous_version=$(sed -n 's/^## \[\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)\].*/\1/p' CHANGELOG.md | head -n1)
if [ -z "$previous_version" ]; then
    printf '%s\n' "error: could not determine the previous version from CHANGELOG.md" >&2
    exit 1
fi

release_date=$(LANG=en_US.UTF-8 date '+%Y-%m-%d')
debian_date=$(LANG=en_US.UTF-8 date '+%a, %d %b %Y %H:%M:%S %z')

# 1. Update Cargo.toml
printf '%s\n' "Updating Cargo.toml..."
sed -i.bak "s/^version = \"[^\"]*\"/version = \"$new_version\"/" Cargo.toml
rm -f Cargo.toml.bak

printf '%s\n' "Updating Cargo.lock workspace package versions..."
lock_tmp=$(mktemp)
awk -v version="$new_version" '
    /^\[\[package\]\]$/ { wayexpand_package = 0 }
    /^name = "wayexpand(-|\")/ { wayexpand_package = 1 }
    wayexpand_package && /^version = "/ {
        sub(/^version = "[^"]*"/, "version = \"" version "\"")
        wayexpand_package = 0
    }
    { print }
' Cargo.lock > "$lock_tmp"
mv "$lock_tmp" Cargo.lock

# 2. Update the human and distro changelogs.
printf '%s\n' "Updating CHANGELOG.md..."
changelog_tmp=$(mktemp)
awk -v version="$new_version" -v date="$release_date" '
    !inserted && /^## \[[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\]/ {
        print "## [" version "] - " date
        print ""
        print "Release v" version ". Move the unreleased entries above into this section before publishing."
        print ""
        inserted = 1
    }
    { print }
' CHANGELOG.md > "$changelog_tmp"
mv "$changelog_tmp" CHANGELOG.md

links_tmp=$(mktemp)
awk -v version="$new_version" -v previous="$previous_version" '
    !inserted && /^\[Unreleased\]:/ {
        print "[Unreleased]: https://github.com/itchyitchy123/wayexpand/compare/v" version "...HEAD"
        print "[" version "]: https://github.com/itchyitchy123/wayexpand/compare/v" previous "...v" version
        inserted = 1
        next
    }
    { print }
' CHANGELOG.md > "$links_tmp"
mv "$links_tmp" CHANGELOG.md

# 3. Update package metadata.
printf '%s\n' "Updating PKGBUILD and wayexpand.spec..."
sed -i.bak "s/^pkgver=.*/pkgver=$new_version/" PKGBUILD
rm -f PKGBUILD.bak
sed -i.bak "s/^Version:        .*/Version:        $new_version/" wayexpand.spec
rm -f wayexpand.spec.bak
metainfo_tmp=$(mktemp)
awk -v version="$new_version" -v date="$release_date" '
    !inserted && /<releases>/ {
        print
        print "    <release version=\"" version "\" date=\"" date "\">"
        print "      <description>"
        print "        <p>Release v" version ". See CHANGELOG.md for details.</p>"
        print "      </description>"
        print "    </release>"
        inserted = 1
        next
    }
    { print }
' io.github.itchyitchy123.WayExpand.metainfo.xml > "$metainfo_tmp"
mv "$metainfo_tmp" io.github.itchyitchy123.WayExpand.metainfo.xml
sed -i.bak "s#<version>[^<]*</version>#<version>$new_version</version>#" \
    desktop/wayexpand-ibus.xml
rm -f desktop/wayexpand-ibus.xml.bak
spec_tmp=$(mktemp)
awk -v version="$new_version" -v date="$release_date" '
    !inserted && /^%changelog$/ {
        print
        print "* " date " Stephan Loesevitz <stephan.loesevitz@gmail.com> - " version "-1"
        print "- Release v" version
        print ""
        inserted = 1
        next
    }
    { print }
' wayexpand.spec > "$spec_tmp"
mv "$spec_tmp" wayexpand.spec

# 4. Update debian/changelog.
printf '%s\n' "Updating debian/changelog..."
(
    printf '%s\n' "wayexpand ($new_version-1) focal; urgency=medium"
    printf '%s\n' ""
    printf '%s\n' "  * Release v$new_version"
    printf '%s\n' ""
    printf '%s\n' " -- Stephan Loesevitz <stephan.loesevitz@gmail.com>  $debian_date"
    printf '%s\n' ""
    cat debian/changelog
) > debian/changelog.tmp
mv debian/changelog.tmp debian/changelog

# Verify the version-bearing fields before making the commit.
test "$(sed -n 's/^version = \"\([^\"]*\)\"/\1/p' Cargo.toml | head -n1)" = "$new_version"
test "$(sed -n 's/^pkgver=//p' PKGBUILD)" = "$new_version"
test "$(sed -n 's/^Version: *//p' wayexpand.spec)" = "$new_version"
test "$(sed -n "s/^wayexpand (\([^ -]*\)-.*/\1/p" debian/changelog | head -n1)" = "$new_version"
test "$(sed -n 's/.*<release version=\"\([^\"]*\)\".*/\1/p' \
    io.github.itchyitchy123.WayExpand.metainfo.xml | head -n1)" = "$new_version"
test "$(sed -n 's/.*<version>\([^<]*\)<\/version>.*/\1/p' \
    desktop/wayexpand-ibus.xml | head -n1)" = "$new_version"
grep -q "^## \[$new_version\]" CHANGELOG.md

# 5. Commit changes with the project maintainer identity.
printf '%s\n' "Committing version updates..."
git add Cargo.toml Cargo.lock CHANGELOG.md debian/changelog PKGBUILD wayexpand.spec \
    io.github.itchyitchy123.WayExpand.metainfo.xml desktop/wayexpand-ibus.xml
git -c user.name='Stephan Loesevitz' -c user.email='stephan.loesevitz@gmail.com' \
    commit -m "release: version $new_version"

# 6. Create tag
printf '%s\n' "Creating git tag v$new_version..."
git -c user.name='Stephan Loesevitz' -c user.email='stephan.loesevitz@gmail.com' \
    tag -a "v$new_version" -m "Release v$new_version"

printf '%s\n' ""
printf '%s\n' "✓ Release v$new_version prepared successfully"
printf '%s\n' ""
printf '%s\n' "ARCHITECTURAL ISSUE (P1): PKGBUILD checksum workflow"
printf '%s\n' "  The release tag currently excludes PKGBUILD checksums because they"
printf '%s\n' "  depend on the GitHub release tarball which is created after tagging."
printf '%s\n' ""
printf '%s\n' "  Current workaround:"
printf '%s\n' "    1. Push the tag to GitHub"
printf '%s\n' "    2. Compute sha256sum of released archive"
printf '%s\n' "    3. Update PKGBUILD and commit (creates stale tag)"
printf '%s\n' ""
printf '%s\n' "  Proper fix (v1.3 refactor): Decouple version-only commit from"
printf '%s\n' "  checksum-bearing release commit. Build artifact early and include"
printf '%s\n' "  checksum before tagging."
printf '%s\n' ""
printf '%s\n' "Next steps:"
printf '%s\n' "  1. Review the commit: git log -1"
printf '%s\n' "  2. Review the tag: git show v$new_version"
printf '%s\n' "  3. Push to GitHub: git push origin main v$new_version"
printf '%s\n' "  4. [WORKAROUND] Recompute PKGBUILD sha256sums for the new source archive"
printf '%s\n' ""
printf '%s\n' "The release workflow will then:"
printf '%s\n' "  - Verify all versions match"
printf '%s\n' "  - Build release binaries"
printf '%s\n' "  - Create a GitHub release with prebuilt binaries"
