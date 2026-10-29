#!/bin/sh
# Install a built WayExpand .deb or .rpm into a disposable system, check the
# installed binaries, reinstall it, and remove it again.
#
# It installs and removes system packages, so it refuses to run unless
# WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE=1 says the system is a throwaway
# container (CI's package-smoke job). It must run as root there.
set -eu

package=${1:?usage: test-package-smoke.sh <wayexpand .deb or .rpm>}
expected_version=${2:-}

if [ "${WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE:-}" != 1 ]; then
    printf '%s\n' 'error: run only in a disposable container (set WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE=1)' >&2
    exit 2
fi
if [ "$(id -u)" -ne 0 ]; then
    printf '%s\n' 'error: package installation needs root inside the container' >&2
    exit 2
fi

case "$package" in
    *.deb)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        install_package() { apt-get install -y -qq "$package"; }
        reinstall_package() { apt-get install -y -qq --reinstall "$package"; }
        remove_package() { apt-get remove -y -qq wayexpand; }
        ;;
    *.rpm)
        install_package() { dnf install -y -q "$package"; }
        reinstall_package() { dnf reinstall -y -q "$package"; }
        remove_package() { dnf remove -y -q wayexpand; }
        ;;
    *)
        printf 'error: not a .deb or .rpm package: %s\n' "$package" >&2
        exit 2
        ;;
esac

binaries='wayexpand wayexpand-daemon wayexpand-action-broker wayexpand-gui wayexpand-ui wayexpand-ibus'

check_installed() {
    for binary in $binaries; do
        path=/usr/bin/$binary
        [ -x "$path" ] || {
            printf 'error: %s is not installed\n' "$path" >&2
            return 1
        }
        # Every shared library the package needs must come from its declared
        # dependencies.
        if ldd "$path" | grep -F 'not found'; then
            printf 'error: %s has unresolved shared libraries\n' "$path" >&2
            return 1
        fi
    done
    for binary in wayexpand wayexpand-daemon wayexpand-gui wayexpand-ui; do
        version=$("/usr/bin/$binary" --version)
        printf '%s\n' "$version"
        if [ -n "$expected_version" ]; then
            case "$version" in
                *" $expected_version"*) ;;
                *)
                    printf 'error: %s does not report version %s\n' "$binary" "$expected_version" >&2
                    return 1
                    ;;
            esac
        fi
    done
    for unit in wayexpand.service wayexpand-input-method.service \
        wayexpand-evdev.service wayexpand-action-broker.service; do
        [ -f "/usr/lib/systemd/user/$unit" ] || {
            printf 'error: user unit %s is missing\n' "$unit" >&2
            return 1
        }
    done
    # The CLI validates the shipped example as an unprivileged user, the way
    # a new user would start from it.
    id wxsmoke >/dev/null 2>&1 || useradd --create-home wxsmoke
    install -o wxsmoke -m 0600 /etc/wayexpand/expansions.toml.example \
        /home/wxsmoke/expansions.toml
    su wxsmoke -s /bin/sh -c 'cd && wayexpand validate expansions.toml'
}

install_package
check_installed
reinstall_package
check_installed
remove_package
for binary in $binaries; do
    if [ -e "/usr/bin/$binary" ]; then
        printf 'error: /usr/bin/%s remains after removal\n' "$binary" >&2
        exit 1
    fi
done
printf 'package smoke test passed: %s\n' "$(basename "$package")"
