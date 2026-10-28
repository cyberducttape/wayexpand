#!/bin/sh
# Source from a package build section to validate/export the source identity
# embedded by release and distribution source publishers.
set -eu

workspace_version=$(sed -n 's/^version = "\([0-9][^"]*\)"/\1/p' Cargo.toml | head -n1)

fail_build_identity() {
    printf '%s\n' "$1" >&2
    exit 1
}

if [ -f debian/wayexpand-build-sha ]; then
    stamped_sha=$(cat debian/wayexpand-build-sha)
    if [ -n "${WAYEXPAND_BUILD_SHA:-}" ] && [ "$WAYEXPAND_BUILD_SHA" != "$stamped_sha" ]; then
        fail_build_identity 'error: WAYEXPAND_BUILD_SHA disagrees with the source stamp'
    fi
    WAYEXPAND_BUILD_SHA=$stamped_sha
    export WAYEXPAND_BUILD_SHA
fi

case ${WAYEXPAND_BUILD_SHA:-} in
    ''|*[!0123456789abcdef]*)
        if [ -z "${WAYEXPAND_BUILD_SHA:-}" ] \
            && git rev-parse --verify HEAD >/dev/null 2>&1; then
            : # Native Git builds derive their identity in the Cargo build script.
        else
            fail_build_identity 'error: package build has no valid source commit identity'
        fi
        ;;
    *)
        if [ "${#WAYEXPAND_BUILD_SHA}" -lt 12 ] || [ "${#WAYEXPAND_BUILD_SHA}" -gt 40 ]; then
            fail_build_identity 'error: package source commit must contain 12 to 40 lowercase hex digits'
        fi
        ;;
esac

if [ -f debian/wayexpand-build-version ]; then
    stamped_version=$(cat debian/wayexpand-build-version)
    if [ -n "${WAYEXPAND_BUILD_RELEASE_VERSION:-}" ] \
        && [ "$WAYEXPAND_BUILD_RELEASE_VERSION" != "$stamped_version" ]; then
        fail_build_identity 'error: WAYEXPAND_BUILD_RELEASE_VERSION disagrees with the source stamp'
    fi
    WAYEXPAND_BUILD_RELEASE_VERSION=$stamped_version
    export WAYEXPAND_BUILD_RELEASE_VERSION
fi

if [ -n "${WAYEXPAND_BUILD_RELEASE_VERSION:-}" ] \
    && [ "$WAYEXPAND_BUILD_RELEASE_VERSION" != "$workspace_version" ]; then
    fail_build_identity 'error: release build version does not match Cargo.toml'
fi

if [ -n "${WAYEXPAND_BUILD_RELEASE_VERSION:-}" ] \
    && [ -z "${WAYEXPAND_BUILD_SHA:-}" ]; then
    fail_build_identity 'error: an exact release version requires an explicit source commit'
fi
