#!/bin/sh
# Compute the version identity of an experimental (uncertified) build of the
# current checkout, and optionally stamp it into the Debian and RPM metadata.
#
# Prints two lines:
#   package_version=<version for dpkg/rpm>
#   label=<the same, using only filename-safe characters>
#
# The package version sorts correctly against certified releases in both dpkg
# and rpm: `+` after an already tagged version (a post-release snapshot), `~`
# before a version that has not been tagged yet (a pre-release snapshot), so a
# later certified release always upgrades an experimental package.
#
# With --apply, debian/changelog and wayexpand.spec are rewritten in place.
# Only use --apply in a disposable CI checkout.
set -eu

apply=0
case "${1:-}" in
    '') ;;
    --apply) apply=1 ;;
    *)
        printf '%s\n' "usage: $0 [--apply]" >&2
        exit 2
        ;;
esac

cargo_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
case "$cargo_version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *)
        printf '%s\n' "error: could not read a release version from Cargo.toml" >&2
        exit 1
        ;;
esac
case "$cargo_version" in
    *[!0-9.]*)
        printf '%s\n' "error: Cargo.toml version must be X.Y.Z for experimental builds: $cargo_version" >&2
        exit 1
        ;;
esac

# The commit date, not the build date, keeps the identity reproducible.
commit_date=$(git show -s --format=%cd --date=format:%Y%m%d HEAD)
commit=$(git rev-parse --short=12 HEAD)
if git rev-parse -q --verify "refs/tags/v$cargo_version" >/dev/null; then
    separator='+'
else
    separator='~'
fi
package_version="${cargo_version}${separator}exp.${commit_date}.g${commit}"
label="${cargo_version}-exp.${commit_date}.g${commit}"

if [ "$apply" -eq 1 ]; then
    sed -i "1s/^wayexpand ([^)]*)/wayexpand (${package_version}-1)/" debian/changelog
    sed -i "s/^Version:\( *\).*/Version:\1${package_version}/" wayexpand.spec
    head -n 1 debian/changelog | grep -F "wayexpand (${package_version}-1)" >/dev/null || {
        printf '%s\n' 'error: could not stamp debian/changelog' >&2
        exit 1
    }
    [ "$(sed -n 's/^Version: *//p' wayexpand.spec)" = "$package_version" ] || {
        printf '%s\n' 'error: could not stamp wayexpand.spec' >&2
        exit 1
    }
fi

printf 'package_version=%s\n' "$package_version"
printf 'label=%s\n' "$label"
