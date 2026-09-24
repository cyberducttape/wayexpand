#!/bin/sh
# Verify that every supported distribution path ships the same runtime assets.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for relative in \
    desktop/wayexpand-ibus.xml \
    systemd/wayexpand-input-method.service \
    systemd/wayexpand-evdev.service \
    udev/71-wayexpand-evdev.rules \
    udev/69-wayexpand-evdev-uaccess.rules; do
    test -f "$project_dir/$relative" || {
        printf '%s\n' "missing required asset: $relative" >&2
        exit 1
    }
done

for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/wayexpand.spec" "$project_dir/.github/workflows/release.yml" \
    "$project_dir/scripts/install-release.sh" "$project_dir/scripts/install-user.sh"; do
    for binary in wayexpand wayexpand-daemon wayexpand-ui wayexpand-gui wayexpand-ibus; do
        grep -q -F "$binary" "$manifest" || {
            printf '%s\n' "manifest omits $binary: $manifest" >&2
            exit 1
        }
    done
done

for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/wayexpand.spec" "$project_dir/.github/workflows/release.yml"; do
    grep -q -F 'wayexpand-ibus.xml' "$manifest" || {
        printf '%s\n' "manifest omits the IBus component: $manifest" >&2
        exit 1
    }
done

# Distro packages may ship the policies as data, but must never install them
# into udev's active rules directory as part of the base package.
for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" "$project_dir/wayexpand.spec"; do
    grep -q -E 'usr/share/wayexpand/udev|%\{_datadir\}/wayexpand/udev' "$manifest" || {
        printf '%s\n' "manifest does not ship inert evdev policies: $manifest" >&2
        exit 1
    }
    if grep -q -E 'usr/(lib|share)/udev/rules.d|%\{_udevrulesdir\}' "$manifest"; then
        printf '%s\n' "manifest installs active udev policy: $manifest" >&2
        exit 1
    fi
done

printf '%s\n' "distribution manifest check passed"
