#!/bin/sh
# Verify that every supported distribution path ships the same runtime assets.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for relative in \
    desktop/wayexpand-ibus.xml \
    systemd/wayexpand-input-method.service \
    systemd/wayexpand-evdev.service \
    systemd/wayexpand-action-broker.service \
    broker.toml.example \
    udev/71-wayexpand-evdev.rules \
    udev/69-wayexpand-evdev-uaccess.rules; do
    test -f "$project_dir/$relative" || {
        printf '%s\n' "missing required asset: $relative" >&2
        exit 1
    }
done

# Debian/Launchpad builds are network-isolated and must consume the vendored
# source archive rather than trying to regenerate dependencies in the chroot.
if grep -q -F 'CARGO_NET_OFFLINE=false cargo vendor' "$project_dir/debian/rules"; then
    printf '%s\n' "Debian rules must not fetch crates during an offline build" >&2
    exit 1
fi
grep -q -F 'require the vendored source archive' "$project_dir/debian/rules" || {
    printf '%s\n' "Debian rules do not fail clearly when vendor/ is absent" >&2
    exit 1
}

for rule in "$project_dir/udev/71-wayexpand-evdev.rules" \
    "$project_dir/udev/69-wayexpand-evdev-uaccess.rules"; do
    grep -q -F 'ENV{ID_INPUT_KEYBOARD}=="?*"' "$rule" || {
        printf '%s\n' "evdev udev rule is not scoped to keyboard-class event nodes: $rule" >&2
        exit 1
    }
done

for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/.github/workflows/release.yml" \
    "$project_dir/scripts/install-release.sh" "$project_dir/scripts/install-user.sh"; do
    for binary in wayexpand wayexpand-daemon wayexpand-action-broker wayexpand-ui wayexpand-gui wayexpand-ibus; do
        grep -q -F "$binary" "$manifest" || {
            printf '%s\n' "manifest omits $binary: $manifest" >&2
            exit 1
        }
    done
done

test -f "$project_dir/wayexpand.spec" || {
    printf '%s\n' 'required Fedora RPM spec is missing' >&2
    exit 1
}
for binary in wayexpand wayexpand-daemon wayexpand-action-broker wayexpand-ui wayexpand-gui wayexpand-ibus; do
    grep -q -F "$binary" "$project_dir/wayexpand.spec" || {
        printf '%s\n' "manifest omits $binary: $project_dir/wayexpand.spec" >&2
        exit 1
    }
done

for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/.github/workflows/release.yml"; do
    grep -q -F 'wayexpand-ibus.xml' "$manifest" || {
        printf '%s\n' "manifest omits the IBus component: $manifest" >&2
        exit 1
    }
done

grep -q -F 'wayexpand-ibus.xml' "$project_dir/wayexpand.spec" || {
    printf '%s\n' "manifest omits the IBus component: $project_dir/wayexpand.spec" >&2
    exit 1
}
grep -q -F -- '-vendored.tar.gz' "$project_dir/wayexpand.spec" || {
    printf '%s\n' 'RPM spec must use the offline-build vendored release source' >&2
    exit 1
}

# Distro packages may ship the policies as data, but must never install them
# into udev's active rules directory as part of the base package.
for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD"; do
    grep -q -E 'usr/share/wayexpand/udev|%\{_datadir\}/wayexpand/udev' "$manifest" || {
        printf '%s\n' "manifest does not ship inert evdev policies: $manifest" >&2
        exit 1
    }
    if grep -q -E 'usr/(lib|share)/udev/rules.d|%\{_udevrulesdir\}' "$manifest"; then
        printf '%s\n' "manifest installs active udev policy: $manifest" >&2
        exit 1
    fi
done

# wayexpand.spec is optional (may be removed if not used for distribution)
if test -f "$project_dir/wayexpand.spec"; then
    grep -q -E 'usr/share/wayexpand/udev|%\{_datadir\}/wayexpand/udev' "$project_dir/wayexpand.spec" || {
        printf '%s\n' "manifest does not ship inert evdev policies: $project_dir/wayexpand.spec" >&2
        exit 1
    }
    if grep -q -E 'usr/(lib|share)/udev/rules.d|%\{_udevrulesdir\}' "$project_dir/wayexpand.spec"; then
        printf '%s\n' "manifest installs active udev policy: $project_dir/wayexpand.spec" >&2
        exit 1
    fi
fi

# Systemd user units use %h/.local/lib/wayexpand/current/bin in the source tree
# for user installs.
# Distro packages install binaries in /usr/bin, so every packaged service must
# be rewritten, including the Action Broker.
package_units='wayexpand-input-method.service wayexpand-evdev.service wayexpand-action-broker.service'
unit_rewrite="sed -i -e 's#%h/.local/lib/wayexpand/current/bin/#/usr/bin/#g' -e 's#%h/.local/bin/#/usr/bin/#g'"
spec_transform=$(awk -v prefix="$unit_rewrite" 'index($0, prefix) == 1 { printing = 1 } printing && /^install/ { exit } printing { print }' "$project_dir/wayexpand.spec")
[ -n "$spec_transform" ] || {
    printf '%s\n' 'RPM spec does not rewrite packaged service binaries to /usr/bin' >&2
    exit 1
}
for unit in $package_units; do
    printf '%s\n' "$spec_transform" | grep -F "$unit" >/dev/null || {
        printf '%s\n' "RPM spec omits packaged unit from /usr/bin rewrite: $unit" >&2
        exit 1
    }
done
grep -F "$unit_rewrite" "$project_dir/debian/rules" >/dev/null || {
    printf '%s\n' 'Debian rules do not rewrite packaged service binaries to /usr/bin' >&2
    exit 1
}

staged_units=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-packaged-units.XXXXXX")
trap 'rm -rf "$staged_units"' EXIT INT TERM
for unit in $package_units; do
    sed -e 's#%h/.local/lib/wayexpand/current/bin/#/usr/bin/#g' -e 's#%h/.local/bin/#/usr/bin/#g' \
        "$project_dir/systemd/$unit" >"$staged_units/$unit"
done
grep -Fx 'ExecStart=/usr/bin/wayexpand-daemon --source=input-method' \
    "$staged_units/wayexpand-input-method.service" >/dev/null || {
    printf '%s\n' 'staged IBus unit does not use /usr/bin/wayexpand-daemon' >&2
    exit 1
}
grep -Fx 'ExecStart=/usr/bin/wayexpand-daemon --source=evdev --backend=libei --allow-evdev-sensitive-fields' \
    "$staged_units/wayexpand-evdev.service" >/dev/null || {
    printf '%s\n' 'staged evdev unit does not use /usr/bin/wayexpand-daemon' >&2
    exit 1
}
grep -Fx 'ExecStart=/usr/bin/wayexpand-action-broker --config %h/.config/wayexpand/broker.toml --socket %t/wayexpand-broker.sock' \
    "$staged_units/wayexpand-action-broker.service" >/dev/null || {
    printf '%s\n' 'staged Action Broker unit does not use /usr/bin/wayexpand-action-broker' >&2
    exit 1
}

# License files must be declared at the same path where each package installs
# them. This catches RPM's easy-to-miss distinction between the source file
# name and the generated %{_licensedir}/%{name}/ path.
if test -f "$project_dir/wayexpand.spec"; then
    grep -q -F '%license %{_licensedir}/%{name}/LICENSE' "$project_dir/wayexpand.spec" || {
        printf '%s\n' "RPM spec does not package the installed license path" >&2
        exit 1
    }
fi
grep -q -F 'usr/share/licenses/wayexpand/LICENSE' "$project_dir/PKGBUILD" || {
    printf '%s\n' "Arch package does not install the license" >&2
    exit 1
}
test -f "$project_dir/debian/copyright" || {
    printf '%s\n' "Debian package is missing debian/copyright" >&2
    exit 1
}

printf '%s\n' "distribution manifest check passed"
