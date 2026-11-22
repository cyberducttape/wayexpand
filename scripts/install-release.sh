#!/bin/sh
# Installs a downloaded WayExpand release tarball for the current user.
# Run this from inside the extracted tarball directory
# (the one containing bin/, systemd/, desktop/, and expansions.toml).
set -eu

release_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
. "$release_dir/scripts/desktop-compatibility.sh"
enable_service=0
service_name=
for argument in "$@"; do
    case "$argument" in
        --enable)
            enable_service=1
            ;;
        --service=wayexpand-input-method.service)
            service_name=wayexpand-input-method.service
            ;;
        --service=wayexpand-evdev.service)
            service_name=wayexpand-evdev.service
            ;;
        --service=wayexpand-control.service)
            service_name=wayexpand-control.service
            ;;
        --help|-h)
            printf '%s\n' "usage: $0 [--enable] [--service=wayexpand-input-method.service|wayexpand-evdev.service|wayexpand-control.service]"
            printf '%s\n' "  --enable   daemon-reload and enable the selected user service"
            printf '%s\n' "  --service  select the service when --enable is used"
            exit 0
            ;;
        *)
            printf '%s\n' "error: unknown option $argument; try --help" >&2
            exit 2
            ;;
    esac
done
if [ "$enable_service" -eq 1 ] && [ -z "$service_name" ]; then
    printf '%s\n' "error: --enable requires an explicit --service selection" >&2
    printf '%s\n' "choose a service only after reviewing \"$HOME/.local/bin/wayexpand\" doctor and accepting its documented limitations" >&2
    exit 2
fi
if [ "$enable_service" -eq 1 ] && ! command -v systemctl >/dev/null 2>&1; then
    printf '%s\n' 'error: systemctl is required for --enable' >&2
    exit 127
fi
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
state_dir="$state_home/wayexpand"
unit_dir="$config_home/systemd/user"
case "$config_home" in
    /*) ;;
    *)
        printf '%s\n' "error: XDG_CONFIG_HOME must be an absolute path" >&2
        exit 1
        ;;
esac
case "$data_home" in
    /*) ;;
    *)
        printf '%s\n' "error: XDG_DATA_HOME must be an absolute path" >&2
        exit 1
        ;;
esac
case "$state_home" in
    /*) ;;
    *)
        printf '%s\n' "error: XDG_STATE_HOME must be an absolute path" >&2
        exit 1
        ;;
esac
if [ "$(id -u)" -eq 0 ]; then
    if [ -n "${SUDO_USER:-}" ]; then
        printf '%s\n' "error: do not run the user installer with sudo; run it as $SUDO_USER" >&2
    else
        printf '%s\n' "error: this is a per-user installer; run it as your desktop user, not root" >&2
    fi
    exit 1
fi
wayexpand_print_desktop_compatibility
# One consent-marker location for the CLI, installers, doctor, GUI and the
# unit: systemd evaluates ConditionPathExists before any XDG environment
# applies, so the marker never follows XDG_CONFIG_HOME. The evdev unit is
# inert without it, and systemd reports a skipped condition as success, so
# refuse rather than report an enabled service that never runs; never create
# the marker here. Show compatibility above before asking users to create it.
evdev_marker="$HOME/.config/wayexpand/evdev-sensitive-fields-ack"
legacy_evdev_marker="$config_dir/evdev-sensitive-fields-ack"
if [ "$enable_service" -eq 1 ] && [ "$service_name" = wayexpand-evdev.service ] \
    && [ ! -e "$evdev_marker" ]; then
    printf '%s\n' "error: wayexpand-evdev.service only starts after you accept that it captures password fields" >&2
    if [ "$legacy_evdev_marker" != "$evdev_marker" ] && [ -e "$legacy_evdev_marker" ]; then
        printf '%s\n' "found your consent at its old location $legacy_evdev_marker; move it to the location the service checks:" >&2
        printf '%s\n' "  install -d -m 0700 $HOME/.config/wayexpand" >&2
        printf '%s\n' "  mv $legacy_evdev_marker $evdev_marker" >&2
    else
        printf '%s\n' "review docs/BACKENDS_SENSITIVE_FIELDS.md, then create the marker and rerun:" >&2
        printf '%s\n' "  install -d -m 0700 $HOME/.config/wayexpand" >&2
        printf '%s\n' "  install -m 0600 /dev/null $evdev_marker" >&2
    fi
    exit 2
fi
for binary in wayexpand-daemon wayexpand wayexpand-action-broker wayexpand-ui wayexpand-gui wayexpand-ibus; do
    if [ ! -x "$release_dir/bin/$binary" ]; then
        printf '%s\n' "error: $release_dir/bin/$binary not found" >&2
        printf '%s\n' "run this script from inside the extracted release tarball" >&2
        exit 1
    fi
done

bin_dir="$HOME/.local/bin"
library_dir="$HOME/.local/lib/wayexpand"
version=$(basename "$release_dir" | sed -n 's/^wayexpand-\([0-9][0-9A-Za-z.+~-]*\)-linux-.*/\1/p')
if [ -z "$version" ]; then
    printf '%s\n' 'error: release directory name must be wayexpand-VERSION-linux-ARCH' >&2
    exit 1
fi
application_dir="$data_home/applications"
metainfo_dir="$data_home/metainfo"
man_dir="$data_home/man/man1"
reject_directory_target() {
    destination=$1
    if [ -d "$destination" ] && [ ! -L "$destination" ]; then
        printf 'error: installation target is a directory: %s\n' "$destination" >&2
        return 1
    fi
}
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    reject_directory_target "$bin_dir/$binary"
done
for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
    reject_directory_target "$unit_dir/$unit"
    reject_directory_target "$unit_dir/$unit.d/10-xdg-paths.conf"
done
reject_directory_target "$application_dir/wayexpand.desktop"
config_path="$config_dir/expansions.toml"
if [ "$enable_service" -eq 1 ]; then
    validation_path=
    if [ -e "$config_path" ] || [ -L "$config_path" ]; then
        validation_path="$config_path"
        printf '%s\n' "Validating existing configuration before upgrade: $config_path"
    elif [ -f "$release_dir/expansions.toml" ]; then
        validation_path="$release_dir/expansions.toml"
        printf '%s\n' "Validating bundled configuration before installation: $validation_path"
    else
        printf '%s\n' 'error: --enable requires an existing configuration or bundled expansions.toml' >&2
        exit 1
    fi
    # Validate before replacing any installed files. An upgrade must not leave
    # a mixed-version installation behind when the active library is invalid.
    "$release_dir/bin/wayexpand" validate "$validation_path"
fi

install -d -m 0700 "$state_dir"
chmod 0700 "$state_dir"

install -d -m 0755 "$library_dir"
stage_dir="$library_dir/.staging-$version-$$"
# Never leave a partial payload behind if staging fails or is interrupted.
trap 'rm -rf -- "$stage_dir"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
install -d -m 0755 "$stage_dir/bin" "$stage_dir/systemd" "$stage_dir/desktop" "$stage_dir/ibus" \
    "$stage_dir/share/wayexpand" \
    "$stage_dir/libexec/wayexpand-certification/bin"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    install -m 0755 "$release_dir/bin/$binary" "$stage_dir/bin/$binary"
done
for helper in wx-e2e-sink wx-e2e-type wx-e2e-eis-relay; do
    install -m 0755 "$release_dir/libexec/wayexpand-certification/bin/$helper" \
        "$stage_dir/libexec/wayexpand-certification/bin/$helper"
done
install -m 0755 "$release_dir/libexec/wayexpand-certification/test-kwin-e2e.sh" \
    "$release_dir/libexec/wayexpand-certification/kwin-e2e-session.sh" \
    "$stage_dir/libexec/wayexpand-certification/"
install -m 0644 "$release_dir/libexec/wayexpand-certification/e2e-lib.sh" \
    "$release_dir/libexec/wayexpand-certification/kwin-e2e-report.sh" \
    "$stage_dir/libexec/wayexpand-certification/"
install -m 0644 "$release_dir/systemd"/*.service "$stage_dir/systemd/"
install -m 0644 "$release_dir/desktop/wayexpand.desktop" "$stage_dir/desktop/wayexpand.desktop"
install -m 0644 "$release_dir/ibus/component/wayexpand-ibus.xml" "$stage_dir/ibus/wayexpand.xml"
install -m 0644 "$release_dir/installer-components.toml" "$stage_dir/share/wayexpand/installer-components.toml"
install -m 0644 "$release_dir/expansions.toml" "$stage_dir/expansions.toml"
printf '%s\n' 'user-local-release-archive' >"$stage_dir/share/wayexpand/install-origin"
# Name the install by its content, not only its version: a corrected archive
# with the same version string must never reuse binaries installed earlier.
payload_hash=$("$release_dir/scripts/hash-install-payload.sh" "$stage_dir")
case "$payload_hash" in
    ''|*[!0-9a-f]*)
        printf '%s\n' 'error: could not determine a safe install payload identity' >&2
        exit 1
        ;;
esac
if [ "${#payload_hash}" -ne 64 ]; then
    printf '%s\n' 'error: could not determine a valid install payload hash' >&2
    exit 1
fi
version="$version-$payload_hash"
if [ ! -e "$library_dir/$version" ] && [ ! -L "$library_dir/$version" ]; then
    mv "$stage_dir" "$library_dir/$version"
else
    if [ -L "$library_dir/$version" ] || [ ! -d "$library_dir/$version" ]; then
        printf '%s\n' "error: existing version path is not a real directory: $library_dir/$version" >&2
        exit 1
    fi
    existing_hash=$("$release_dir/scripts/hash-install-payload.sh" "$library_dir/$version") || {
        printf '%s\n' "error: existing version directory is incomplete or unsafe: $library_dir/$version" >&2
        exit 1
    }
    if [ "$existing_hash" != "$payload_hash" ]; then
        printf '%s\n' "error: existing version directory does not match its payload identity: $library_dir/$version" >&2
        exit 1
    fi
    rm -rf "$stage_dir"
fi
service_was_enabled=0
if [ "$enable_service" -eq 1 ] && systemctl --user is-enabled "$service_name" >/dev/null 2>&1; then
    service_was_enabled=1
fi
previous_version=$(readlink "$library_dir/current" 2>/dev/null || true)
rollback_dir="$state_dir/.install-rollback-$$"
install -d -m 0700 "$rollback_dir/items" "$rollback_dir/present"
snapshot_target() {
    snapshot_key=$1
    snapshot_path=$2
    if [ -e "$snapshot_path" ] || [ -L "$snapshot_path" ]; then
        cp -a -- "$snapshot_path" "$rollback_dir/items/$snapshot_key"
        : >"$rollback_dir/present/$snapshot_key"
    fi
}
restore_target() {
    restore_key=$1
    restore_path=$2
    if [ -f "$rollback_dir/present/$restore_key" ]; then
        rm -rf -- "$restore_path" || return 1
        mkdir -p -- "${restore_path%/*}" || return 1
        cp -a -- "$rollback_dir/items/$restore_key" "$restore_path" || return 1
    else
        rm -rf -- "$restore_path" || return 1
    fi
}
snapshot_external_files() {
    for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
        snapshot_target "bin-$binary" "$bin_dir/$binary"
    done
    for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
        snapshot_target "unit-$unit" "$unit_dir/$unit"
        snapshot_target "dropin-$unit" "$unit_dir/$unit.d/10-xdg-paths.conf"
    done
    snapshot_target desktop "$application_dir/wayexpand.desktop"
    snapshot_target metainfo "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
    snapshot_target man "$man_dir/wayexpand.1"
    for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
        snapshot_target "icon-$size" "$data_home/icons/hicolor/$size/apps/wayexpand.png"
    done
    snapshot_target config "$config_path"
    snapshot_target broker-config "$config_dir/broker.toml"
    snapshot_target legacy-dropin "$unit_dir/wayexpand-action-broker.service.d/10-state-directory.conf"
    snapshot_target previous-install "$state_dir/previous-install"
    snapshot_target stale-ibus "$data_home/ibus/component/wayexpand.xml"
}
restore_external_files() {
    restore_errors=0
    for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
        restore_target "bin-$binary" "$bin_dir/$binary" || restore_errors=1
    done
    for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
        restore_target "unit-$unit" "$unit_dir/$unit" || restore_errors=1
        restore_target "dropin-$unit" "$unit_dir/$unit.d/10-xdg-paths.conf" || restore_errors=1
    done
    restore_target desktop "$application_dir/wayexpand.desktop" || restore_errors=1
    restore_target metainfo "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml" || restore_errors=1
    restore_target man "$man_dir/wayexpand.1" || restore_errors=1
    for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
        restore_target "icon-$size" "$data_home/icons/hicolor/$size/apps/wayexpand.png" || restore_errors=1
    done
    restore_target config "$config_path" || restore_errors=1
    restore_target broker-config "$config_dir/broker.toml" || restore_errors=1
    restore_target legacy-dropin "$unit_dir/wayexpand-action-broker.service.d/10-state-directory.conf" || restore_errors=1
    restore_target previous-install "$state_dir/previous-install" || restore_errors=1
    restore_target stale-ibus "$data_home/ibus/component/wayexpand.xml" || restore_errors=1
    return "$restore_errors"
}
installer_checkpoint() {
    if [ "${WAYEXPAND_INSTALL_TEST_FAIL_AFTER:-}" = "$1" ]; then
        printf 'error: injected installer failure after %s\n' "$1" >&2
        return 1
    fi
}
snapshot_external_files
transaction_started=1
next_link="$library_dir/.current-$$"
version_switch_started=0
restore_previous_version() {
    if [ -n "$previous_version" ]; then
        rollback_link="$library_dir/.rollback-$$"
        if ln -s "$previous_version" "$rollback_link"; then
            mv -Tf "$rollback_link" "$library_dir/current"
        else
            printf '%s\n' 'error: could not prepare installer rollback link' >&2
            return 1
        fi
    else
        rm -f "$library_dir/current"
    fi
}
rollback_on_install_failure() {
    install_status=$?
    if [ "$install_status" -ne 0 ]; then
        rollback_errors=0
        if [ "$version_switch_started" -eq 1 ]; then
            printf '%s\n' 'error: installation failed during or after switching to the staged version; restoring the previous version' >&2
            restore_previous_version || rollback_errors=1
        fi
        if [ "$transaction_started" -eq 1 ]; then
            restore_external_files || rollback_errors=1
        fi
        rm -f "$next_link"
        rm -f "$state_dir/previous-install-$$"
        if [ "$version_switch_started" -eq 1 ] && [ "$enable_service" -eq 1 ]; then
            systemctl --user daemon-reload || true
            if [ "$service_was_enabled" -eq 1 ]; then
                systemctl --user restart "$service_name" || true
            else
                systemctl --user disable --now "$service_name" >/dev/null 2>&1 || true
            fi
        fi
        if [ "$rollback_errors" -eq 0 ]; then
            rm -rf -- "$rollback_dir"
        else
            printf 'error: rollback was incomplete; recovery snapshots remain at %s\n' "$rollback_dir" >&2
        fi
    else
        rm -rf -- "$rollback_dir"
    fi
    return "$install_status"
}
trap rollback_on_install_failure EXIT
if [ -n "$previous_version" ]; then
    install -d -m 0700 "$state_dir"
    rollback_tmp="$state_dir/previous-install-$$"
    (umask 077; printf '%s\n' "$previous_version" > "$rollback_tmp")
    mv -f "$rollback_tmp" "$state_dir/previous-install"
fi
ln -s "$version" "$next_link"
# Mark the transaction before mv: if a signal or an I/O error interrupts the
# atomic rename, rollback is safe whether the old or new link is visible.
version_switch_started=1
mv -Tf "$next_link" "$library_dir/current"
install -d -m 0755 "$bin_dir" "$unit_dir" "$application_dir"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    ln -sfnT "$library_dir/current/bin/$binary" "$bin_dir/$binary"
done
installer_checkpoint binaries
for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
    ln -sfnT "$library_dir/current/systemd/$unit" "$unit_dir/$unit"
done
installer_checkpoint services
# ibus-daemon never reads per-user component directories, so a manifest
# there cannot register the engine. Remove the link older installers made;
# `wayexpand setup --backend ibus` installs the component where IBus reads it.
stale_ibus_component="$data_home/ibus/component/wayexpand.xml"
if [ -L "$stale_ibus_component" ]; then
    case $(readlink "$stale_ibus_component") in
        "$library_dir"/*) rm -f -- "$stale_ibus_component" ;;
    esac
fi
"$release_dir/scripts/install-xdg-systemd-dropins.sh" \
    "$unit_dir" "$config_home" "$config_dir" "$state_home" "$state_dir" "$library_dir/current/bin"
installer_checkpoint dropins
ln -sfnT "$library_dir/current/desktop/wayexpand.desktop" "$application_dir/wayexpand.desktop"
installer_checkpoint desktop
if [ -f "$release_dir/io.github.cyberducttape.WayExpand.metainfo.xml" ]; then
    install -Dm644 "$release_dir/io.github.cyberducttape.WayExpand.metainfo.xml" \
        "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
fi
installer_checkpoint metadata
if [ -f "$release_dir/docs/wayexpand.1" ]; then
    install -Dm644 "$release_dir/docs/wayexpand.1" "$man_dir/wayexpand.1"
fi

icon_base="$data_home/icons/hicolor"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    icon_src="$release_dir/icons/hicolor/$size/apps/wayexpand.png"
    if [ -f "$icon_src" ]; then
        install -Dm644 "$icon_src" "$icon_base/$size/apps/wayexpand.png"
    fi
done
installer_checkpoint icons

if [ -e "$config_path" ] || [ -L "$config_path" ]; then
    printf '%s\n' "Keeping existing configuration: $config_path"
elif [ -f "$release_dir/expansions.toml" ]; then
    install -Dm600 "$release_dir/expansions.toml" "$config_path"
    printf '%s\n' "Installed example configuration: $config_path"
fi
installer_checkpoint configuration
broker_config="$config_dir/broker.toml"
if [ -e "$broker_config" ] || [ -L "$broker_config" ]; then
    printf '%s\n' "Keeping existing broker policy: $broker_config"
elif [ -f "$release_dir/broker.toml.example" ]; then
    install -Dm600 "$release_dir/broker.toml.example" "$broker_config"
    printf '%s\n' "Installed broker policy template: $broker_config"
fi
installer_checkpoint broker_configuration

printf '%s\n' "Installed binaries in $bin_dir"
printf '%s\n' "Installed user units in $unit_dir"
printf '%s\n' "Installed desktop entry in $application_dir"
printf '%s\n' "Staged IBus component in $library_dir/current/ibus (register it with \"$bin_dir/wayexpand\" setup --backend ibus)"
if [ -d "$release_dir/icons" ]; then
    printf '%s\n' "Installed application icon in $icon_base"
fi
printf '%s\n' "Next action: open the setup guide with: \"$bin_dir/wayexpand-gui\""
printf '%s\n' "You can also launch WayExpand from your desktop application menu."
printf '%s\n' ""
printf '%s\n' "Optional command-line setup:"
printf '%s\n' "  export PATH=\"$bin_dir:\$PATH\""
case "${SHELL##*/}" in
    bash)
        printf '%s\n' "For future Bash terminals, run once: printf '\\nexport PATH=\"$bin_dir:\$PATH\"\\n' >> \"$HOME/.bashrc\"" ;;
    zsh)
        printf '%s\n' "For future Zsh terminals, run once: printf '\\nexport PATH=\"$bin_dir:\$PATH\"\\n' >> \"$HOME/.zshrc\"" ;;
    fish)
        printf '%s\n' "For future Fish terminals, run once: fish_add_path \"$bin_dir\"" ;;
    *)
        printf '%s\n' "To keep this PATH in future terminals, see $release_dir/README.md" ;;
esac
printf '%s\n' "  \"$bin_dir/wayexpand\" setup     # guided setup; checks this session and explains the next step"
printf '%s\n' "  \"$bin_dir/wayexpand\" doctor    # diagnose an unavailable route"
printf '%s\n' "  # For named actions, edit $config_dir/broker.toml and enable:"
printf '%s\n' "  systemctl --user enable --now wayexpand-action-broker.service"
printf '%s\n' "  \"$bin_dir/wayexpand\" edit      # edit snippets"
if [ -f "$release_dir/polkit/org.wayexpand.action.policy" ] &&
    [ ! -f /usr/share/polkit-1/actions/org.wayexpand.action.policy ]; then
    printf '%s\n' "Broker actions require administrator installation of the polkit policy:"
    printf '%s\n' "  sudo install -m 0644 \"$release_dir/polkit/org.wayexpand.action.policy\" /usr/share/polkit-1/actions/"
fi
printf '%s\n' "Setup never grants raw-input permissions or portal consent. If it reports"
printf '%s\n' "no safe automatic path, review \"$bin_dir/wayexpand\" doctor and the documented explicit modes."
printf '%s\n' "To remove this installation later, run \"$release_dir/scripts/uninstall-user.sh\"."

if command -v systemctl >/dev/null 2>&1; then
    active_upgrade=0
    for active_service in wayexpand-input-method.service wayexpand-evdev.service wayexpand-action-broker.service; do
        if systemctl --user is-active --quiet "$active_service" 2>/dev/null; then
            active_upgrade=1
        fi
    done
    if [ "$active_upgrade" -eq 1 ]; then
        printf '%s\n' 'An active WayExpand service was detected. Runtime process check:'
        if runtime_check_output=$("$bin_dir/wayexpand" upgrade --check 2>&1); then
            printf '%s\n' "$runtime_check_output"
            case "$runtime_check_output" in
                *'Action: restart the affected service'*|*'Action: restart the active Action Broker service'*)
                    printf '%s\n' 'After reviewing the process details above, apply the restart and health check with:'
                    printf '  "%s/wayexpand" upgrade --apply\n' "$bin_dir"
                    ;;
                *)
                    printf '%s\n' 'The runtime check did not identify a managed daemon version mismatch; inspect the status above before restarting anything.'
                    ;;
            esac
        else
            printf '%s\n' "$runtime_check_output"
            printf '%s\n' "Could not complete the runtime check. Inspect the output and rerun \"$bin_dir/wayexpand\" upgrade --check before applying."
        fi
    fi
fi

if [ "$enable_service" -eq 1 ]; then
    "$bin_dir/wayexpand" validate "$config_path"
    printf '%s\n' "Enabling user service: $service_name"
    activation_ok=1
    systemctl --user daemon-reload || activation_ok=0
    if [ "$activation_ok" -eq 1 ]; then
        systemctl --user enable "$service_name" || activation_ok=0
    fi
    if [ "$activation_ok" -eq 1 ]; then
        systemctl --user restart "$service_name" || activation_ok=0
    fi
    if [ "$activation_ok" -eq 1 ]; then
        systemctl --user is-active --quiet "$service_name" || activation_ok=0
    fi
    if [ "$activation_ok" -ne 1 ]; then
        printf '%s\n' "error: unable to activate $service_name; restoring the previous version when available" >&2
        exit 1
    fi
    printf '%s\n' "Enabled $service_name"
fi

# Keep only the active and previous payloads. Each build identity is a full
# copy of the binaries, so unpruned upgrades would grow without bound.
for entry in "$library_dir"/*; do
    name=${entry##*/}
    if [ -L "$entry" ] || [ ! -d "$entry" ] || [ "$name" = "$version" ] ||
        [ "$name" = "${previous_version##*/}" ]; then
        continue
    fi
    rm -rf -- "$entry"
done
