#!/bin/sh
# Install a built WayExpand .deb or .rpm into a disposable system, check the
# installed binaries, reinstall it, and remove it again.
#
# It installs and removes system packages, so it requires both an explicit
# WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE=1 opt-in and a Docker/Podman container
# marker. The marker prevents the opt-in alone from enabling package changes
# on a host; it is a guard against mistakes, not a security boundary against
# root. It must run as root inside the disposable CI container.
set -eu

stage='initializing'
on_exit() {
    exit_code=$?
    if [ "$exit_code" -ne 0 ]; then
        printf 'error: package smoke failed during %s (exit %s)\n' \
            "$stage" "$exit_code" >&2
    fi
}
trap on_exit EXIT

stage='reading package arguments'
package=${1:?usage: test-package-smoke.sh <wayexpand .deb or .rpm> <version> [previous package]}
expected_version=${2:-}
previous_package=${3:-}

stage='checking disposable-container opt-in'
if [ "${WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE:-}" != 1 ]; then
    printf '%s\n' 'error: run only in a disposable container (set WAYEXPAND_PACKAGE_SMOKE_DISPOSABLE=1)' >&2
    exit 2
fi
if [ ! -e /.dockerenv ] && [ ! -e /run/.containerenv ]; then
    printf '%s\n' 'error: package smoke test requires a detected Docker or Podman container' >&2
    exit 2
fi
if [ "$(id -u)" -ne 0 ]; then
    printf '%s\n' 'error: package installation needs root inside the container' >&2
    exit 2
fi

stage='checking smoke-test tools'
for tool in getent id install ldd python3 sha256sum su useradd; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'error: required smoke-test tool is missing: %s\n' "$tool" >&2
        exit 2
    fi
done

case "$package" in
    *.deb)
        export DEBIAN_FRONTEND=noninteractive
        script_directory=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
        sh "$script_directory/use-https-ubuntu-mirrors.sh"
        apt-get update -qq
        # APT treats `dist/package.deb` as a package selector with a release
        # qualifier. Prefix relative archive paths with `./` so it recognizes
        # them as local files; absolute paths are already unambiguous.
        case "$package" in
            /*) package_path=$package ;;
            *) package_path=./$package ;;
        esac
        case "$previous_package" in
            /*) previous_package_path=$previous_package ;;
            *) previous_package_path=./$previous_package ;;
        esac
        install_package() { apt-get install -y -q "$package_path"; }
        reinstall_package() { apt-get install -y -q --reinstall "$package_path"; }
        install_previous_package() { apt-get install -y -q "$previous_package_path"; }
        rollback_package() { apt-get install -y -q --allow-downgrades "$previous_package_path"; }
        remove_package() { apt-get remove -y -qq wayexpand; }
        ;;
    *.rpm)
        install_package() { dnf install -y -q "$package"; }
        reinstall_package() { dnf reinstall -y -q "$package"; }
        install_previous_package() { dnf install -y -q "$previous_package"; }
        rollback_package() { rpm -U --oldpackage "$previous_package"; }
        remove_package() { dnf remove -y -q wayexpand; }
        ;;
    *)
        printf 'error: not a .deb or .rpm package: %s\n' "$package" >&2
        exit 2
        ;;
esac

binaries='wayexpand wayexpand-daemon wayexpand-action-broker wayexpand-gui wayexpand-ui wayexpand-ibus'

check_installed() {
    check_version=$expected_version
    if [ "$#" -gt 0 ]; then
        check_version=$1
    fi
    stage='checking installed executables and shared libraries'
    printf '%s\n' 'Checking installed executables and shared libraries'
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
    stage='checking installed binary versions'
    printf '%s\n' 'Checking installed binary versions'
    for binary in wayexpand wayexpand-daemon wayexpand-gui wayexpand-ui; do
        version=$("/usr/bin/$binary" --version)
        printf '%s\n' "$version"
        if [ -n "$check_version" ]; then
            case "$version" in
                *" $check_version"*) ;;
                *)
                    printf 'error: %s does not report version %s\n' "$binary" "$check_version" >&2
                    return 1
                    ;;
            esac
        fi
    done
    stage='checking installed user service units'
    printf '%s\n' 'Checking installed user service units'
    # Packages ship the system-wide routes; the generic wayexpand.service
    # belongs to the per-user installer.
    for unit in wayexpand-input-method.service wayexpand-evdev.service \
        wayexpand-action-broker.service; do
        [ -f "/usr/lib/systemd/user/$unit" ] || {
            printf 'error: user unit %s is missing\n' "$unit" >&2
            return 1
        }
    done
    stage='checking IBus registration and default user configuration'
    printf '%s\n' 'Checking IBus registration and default user configuration'
    # The CLI validates the shipped example as an unprivileged user, the way
    # a new user would start from it.
    id wxsmoke >/dev/null 2>&1 || useradd --create-home wxsmoke
    id wxsmoke2 >/dev/null 2>&1 || useradd --create-home wxsmoke2
    package_manifest=/usr/share/ibus/component/wayexpand-ibus.xml
    package_manifest_hash=$(sha256sum "$package_manifest" | cut -d ' ' -f 1)
    python3 - "$package_manifest" <<'PY'
import sys
import xml.etree.ElementTree as ET

manifest = ET.parse(sys.argv[1]).getroot()
executables = [element.text for element in manifest.findall("./exec")]
if executables != ["wayexpand-ibus"]:
    raise SystemExit(f"unexpected IBus engine executable in {sys.argv[1]}: {executables!r}")
PY
    install -D -o wxsmoke -m 0600 /etc/wayexpand/expansions.toml.example \
        /home/wxsmoke/.config/wayexpand/expansions.toml
    wxsmoke_home=$(getent passwd wxsmoke | cut -d: -f6)
    stage='validating the packaged configuration as an unprivileged user'
    # $1 must expand in the target user's `su` shell, not in this installer.
    # shellcheck disable=SC2016
    su wxsmoke -s /bin/sh -c 'wayexpand config validate "$1"' wayexpand \
        "$wxsmoke_home/.config/wayexpand/expansions.toml"
    stage='checking installation readiness for the default user'
    install_report=$(su wxsmoke -s /bin/sh -c 'wayexpand install-check --json' 2>/dev/null || true)
    printf '%s\n' "$install_report" | python3 -c '
import json, sys
report = json.load(sys.stdin)
checks = {check["component"]: check for check in report["checks"]}
assert report["ready"] is True, checks
for component in ("cli", "gui", "daemon", "desktop", "ibus", "ibus_engine_executable",
                  "gnome_extension", "installer_contract", "default_config_template", "control_service",
                  "input_method_service", "active_configuration", "xdg_config_home", "xdg_data_home"):
    assert checks[component]["status"] == "pass", (component, checks.get(component))
'
    second_config=/tmp/wayexpand-wxsmoke2-config
    second_data=/tmp/wayexpand-wxsmoke2-data
    stage='checking installation readiness with custom XDG paths'
    install -d -o wxsmoke2 -m 0700 "$second_config/wayexpand" "$second_data"
    install -o wxsmoke2 -m 0600 /etc/wayexpand/expansions.toml.example \
        "$second_config/wayexpand/expansions.toml"
    second_report=$(su wxsmoke2 -s /bin/sh -c \
        "XDG_CONFIG_HOME=$second_config XDG_DATA_HOME=$second_data wayexpand install-check --json" \
        2>/dev/null || true)
    printf '%s\n' "$second_report" | python3 -c '
import json, sys
report = json.load(sys.stdin)
checks = {check["component"]: check for check in report["checks"]}
assert report["ready"] is True, checks
for component in ("ibus", "ibus_engine_executable", "gnome_extension", "installer_contract",
                  "default_config_template", "active_configuration", "xdg_config_home",
                  "xdg_data_home"):
    assert checks[component]["status"] == "pass", (component, checks.get(component))
'
    printf '%s\n' 'Checking second-user XDG configuration and IBus manifest integrity'
    stage='verifying that reinstall preserved the shared IBus manifest'
    if [ "$(sha256sum "$package_manifest" | cut -d ' ' -f 1)" != "$package_manifest_hash" ]; then
        printf '%s\n' 'error: reinstall changed the shared IBus component manifest' >&2
        return 1
    fi
    rm -rf -- "$second_config" "$second_data"
}

printf '%s\n' 'Installing package for the first time'
stage='installing the package for the first time'
if [ -n "$previous_package" ]; then
    stage='installing the previous package version'
    printf 'Installing previous package: %s\n' "$(basename "$previous_package")"
    install_previous_package
    previous_version=$(/usr/bin/wayexpand --version)
    printf '%s\n' "$previous_version"
    if [ -n "$expected_version" ]; then
        case "$previous_version" in
            *" $expected_version"*)
                printf '%s\n' 'error: previous package unexpectedly has the candidate version' >&2
                exit 1
                ;;
        esac
    fi
    [ -x /usr/bin/wayexpand ] || {
        printf '%s\n' 'error: previous package did not install the WayExpand CLI' >&2
        exit 1
    }
    stage='upgrading from the previous package version'
    printf '%s\n' 'Upgrading to the candidate package'
    install_package
else
    install_package
fi
check_installed
if [ -n "$previous_package" ]; then
    stage='rolling back to the previous package version'
    printf '%s\n' 'Rolling back to the previous package'
    rollback_package
    rollback_version=$(/usr/bin/wayexpand --version)
    case "$rollback_version" in
        *" $expected_version"*)
            printf '%s\n' 'error: rollback left the candidate package installed' >&2
            exit 1
            ;;
    esac
    [ -x /usr/bin/wayexpand ] || {
        printf '%s\n' 'error: rollback removed the WayExpand CLI' >&2
        exit 1
    }
    stage='re-upgrading after rollback'
    printf '%s\n' 'Re-upgrading to the candidate package'
    install_package
    check_installed
fi
printf '%s\n' 'Reinstalling package and checking idempotence'
stage='reinstalling the package'
reinstall_package
check_installed
printf '%s\n' 'Removing package and checking executable cleanup'
stage='removing the package and checking executable cleanup'
remove_package
for binary in $binaries; do
    if [ -e "/usr/bin/$binary" ]; then
        printf 'error: /usr/bin/%s remains after removal\n' "$binary" >&2
        exit 1
    fi
done
printf 'package smoke test passed: %s\n' "$(basename "$package")"
