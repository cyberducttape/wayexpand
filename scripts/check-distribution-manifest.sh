#!/bin/sh
# Verify that every supported distribution path ships the same runtime assets.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
for relative in \
    desktop/wayexpand-ibus.xml \
    gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/metadata.json \
    gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/extension.js \
    gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/notification-queue.js \
    installer-components.toml \
    systemd/wayexpand-input-method.service \
    systemd/wayexpand-evdev.service \
    systemd/wayexpand-control.service \
    systemd/wayexpand-action-broker.service \
    scripts/test-kwin-e2e.sh \
    scripts/kwin-e2e-session.sh \
    scripts/kwin-e2e-report.sh \
    scripts/e2e-lib.sh \
    broker.toml.example \
    udev/71-wayexpand-evdev.rules \
    udev/69-wayexpand-evdev-uaccess.rules; do
    test -f "$project_dir/$relative" || {
        printf '%s\n' "missing required asset: $relative" >&2
        exit 1
    }
done

# The machine-readable component contract is shipped by every supported
# package path, and its required payload entries must appear in each recipe.
for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/wayexpand.spec" "$project_dir/scripts/install-user.sh" \
    "$project_dir/scripts/install-release.sh" "$project_dir/scripts/package-release-binaries.sh"; do
    grep -q -F 'installer-components.toml' "$manifest" || {
        printf '%s\n' "installer does not ship the shared component contract: $manifest" >&2
        exit 1
    }
done
for component in cli gui daemon ibus desktop icon default_config_template control_service input_method_service; do
    grep -q -E "^\\[components\\.${component}\\]$" "$project_dir/installer-components.toml" || {
        printf '%s\n' "shared installer contract is missing required component: $component" >&2
        exit 1
    }
    grep -A3 -E "^\\[components\\.${component}\\]$" "$project_dir/installer-components.toml" | grep -q '^required = true$' || {
        printf '%s\n' "shared installer contract must mark $component required" >&2
        exit 1
    }
done
ibus_contract=$(sed -n '/^\[components\.ibus\]$/,/^$/p' "$project_dir/installer-components.toml")
printf '%s\n' "$ibus_contract" | grep -q '^registration = "system-wide component directory selected by the running ibus-daemon;' || {
    printf '%s\n' 'IBus contract must describe system-wide daemon registration' >&2
    exit 1
}
printf '%s\n' "$ibus_contract" | grep -q '^user_registration = false$' || {
    printf '%s\n' 'IBus contract must explicitly disallow per-user registration' >&2
    exit 1
}
printf '%s\n' "$ibus_contract" | grep -q '^user_install_exec = "/usr/local/libexec/wayexpand/wayexpand-ibus"$' || {
    printf '%s\n' 'IBus user-install executable contract is missing or inconsistent' >&2
    exit 1
}
if printf '%s\n' "$ibus_contract" | grep -q '\.local/share/ibus/component'; then
    printf '%s\n' 'IBus contract must not advertise an ordinary per-user component directory' >&2
    exit 1
fi
for installer in "$project_dir/scripts/install-user.sh" "$project_dir/scripts/install-release.sh"; do
    grep -q -F '"$stage_dir/expansions.toml"' "$installer" || {
        printf '%s\n' "user installer does not stage the default configuration template: $installer" >&2
        exit 1
    }
done
for contract_field in current_link config user_units binary_links uninstall upgrade path_note; do
    grep -q "^${contract_field} =" "$project_dir/installer-components.toml" || {
        printf '%s\n' "installer contract is missing lifecycle field: $contract_field" >&2
        exit 1
    }
done

for manifest in "$project_dir/debian/rules" "$project_dir/PKGBUILD" \
    "$project_dir/wayexpand.spec" "$project_dir/.github/workflows/release.yml" \
    "$project_dir/scripts/install-user.sh"; do
    grep -q -F 'wayexpand-e2e' "$manifest" || {
        printf '%s\n' "manifest omits the isolated certification helper build: $manifest" >&2
        exit 1
    }
done

for helper in wx-e2e-sink wx-e2e-type wx-e2e-eis-relay e2e-lib.sh kwin-e2e-report.sh; do
    for manifest in "$project_dir/PKGBUILD" "$project_dir/wayexpand.spec" \
        "$project_dir/debian/rules" "$project_dir/scripts/install-user.sh" \
        "$project_dir/scripts/install-release.sh" "$project_dir/scripts/package-release-binaries.sh"; do
        grep -q -F "$helper" "$manifest" || {
            printf '%s\n' "manifest omits certification helper $helper: $manifest" >&2
            exit 1
        }
    done
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
grep -q -F '. scripts/load-build-identity.sh' "$project_dir/wayexpand.spec" || {
    printf '%s\n' 'RPM build and test sections must load the stamped build identity' >&2
    exit 1
}
test -f "$project_dir/scripts/load-build-identity.sh" || {
    printf '%s\n' 'RPM build identity loader is missing' >&2
    exit 1
}
grep -q -F 'wayexpand-build-sha' "$project_dir/scripts/build-release-packages.sh" || {
    printf '%s\n' 'RPM package source does not carry its source commit identity' >&2
    exit 1
}
grep -q -F 'WAYEXPAND_BUILD_RELEASE_VERSION=' "$project_dir/.github/workflows/release.yml" || {
    printf '%s\n' 'release workflow does not stamp the exact release build version' >&2
    exit 1
}
test "$(grep -Fc 'name: Set exact release build identity' "$project_dir/.github/workflows/release.yml")" -eq 2 || {
    printf '%s\n' 'both native architecture release builds must use the exact release identity' >&2
    exit 1
}
test "$(grep -Fc 'target/release/wayexpand-daemon --version' "$project_dir/.github/workflows/release.yml")" -eq 2 || {
    printf '%s\n' 'both native architecture release builds must verify daemon version and commit' >&2
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
package_units='wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service'
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
grep -Fx 'ExecStart=/usr/bin/wayexpand-daemon --source=control --backend=libei' \
    "$staged_units/wayexpand-control.service" >/dev/null || {
    printf '%s\n' 'staged control unit does not use /usr/bin/wayexpand-daemon' >&2
    exit 1
}
grep -Fx 'ExecStart=/usr/bin/wayexpand-action-broker --config %h/.config/wayexpand/broker.toml --socket %t/wayexpand-broker.sock' \
    "$staged_units/wayexpand-action-broker.service" >/dev/null || {
    printf '%s\n' 'staged Action Broker unit does not use /usr/bin/wayexpand-action-broker' >&2
    exit 1
}

# GNOME window tracking needs the Shell extension installed system-wide by
# every distribution package.
for manifest in debian/rules wayexpand.spec PKGBUILD; do
    grep -F 'gnome-shell/extensions/wayexpand-window-tracker@cyberducttape.github.io' "$project_dir/$manifest" >/dev/null || {
        printf '%s\n' "$manifest does not install the GNOME Shell extension" >&2
        exit 1
    }
done

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
