#!/bin/sh
# Installs a downloaded WayExpand release tarball for the current user.
# Run this from inside the extracted tarball directory
# (the one containing bin/, systemd/, desktop/, and expansions.toml).
set -eu

release_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
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
        --help|-h)
            printf '%s\n' "usage: $0 [--enable] [--service=wayexpand-input-method.service|wayexpand-evdev.service]"
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
    printf '%s\n' "choose a service only after reviewing wayexpand doctor and accepting its documented limitations" >&2
    exit 2
fi
if [ "$enable_service" -eq 1 ] && ! command -v systemctl >/dev/null 2>&1; then
    printf '%s\n' 'error: systemctl is required for --enable' >&2
    exit 127
fi
if [ "$(id -u)" -eq 0 ]; then
    if [ -n "${SUDO_USER:-}" ]; then
        printf '%s\n' "error: do not run the user installer with sudo; run it as $SUDO_USER" >&2
    else
        printf '%s\n' "error: this is a per-user installer; run it as your desktop user, not root" >&2
    fi
    exit 1
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
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
state_dir="$state_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$HOME/.local/share/applications"
metainfo_dir="$HOME/.local/share/metainfo"
man_dir="$HOME/.local/share/man/man1"
case "$config_home" in
    /*) ;;
    *)
        printf '%s\n' "error: XDG_CONFIG_HOME must be an absolute path" >&2
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
install -d -m 0755 "$stage_dir/bin" "$stage_dir/systemd" "$stage_dir/desktop" "$stage_dir/ibus"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    install -m 0755 "$release_dir/bin/$binary" "$stage_dir/bin/$binary"
done
install -m 0644 "$release_dir/systemd"/*.service "$stage_dir/systemd/"
install -m 0644 "$release_dir/desktop/wayexpand.desktop" "$stage_dir/desktop/wayexpand.desktop"
install -m 0644 "$release_dir/ibus/component/wayexpand-ibus.xml" "$stage_dir/ibus/wayexpand.xml"
if [ ! -d "$library_dir/$version" ]; then
    mv "$stage_dir" "$library_dir/$version"
else
    for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
        if [ ! -x "$library_dir/$version/bin/$binary" ]; then
            printf '%s\n' "error: existing version directory is incomplete: $library_dir/$version" >&2
            exit 1
        fi
    done
    rm -rf "$stage_dir"
fi
service_was_enabled=0
if [ "$enable_service" -eq 1 ] && systemctl --user is-enabled "$service_name" >/dev/null 2>&1; then
    service_was_enabled=1
fi
previous_version=$(readlink "$library_dir/current" 2>/dev/null || true)
next_link="$library_dir/.current-$$"
ln -s "$version" "$next_link"
mv -Tf "$next_link" "$library_dir/current"
install -d -m 0755 "$bin_dir"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    ln -sfn "$library_dir/current/bin/$binary" "$bin_dir/$binary"
done
install -d -m 0755 "$unit_dir" "$HOME/.local/share/ibus/component" "$application_dir"
for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-action-broker.service; do
    ln -sfn "$library_dir/current/systemd/$unit" "$unit_dir/$unit"
done
ln -sfn "$library_dir/current/ibus/wayexpand.xml" \
    "$HOME/.local/share/ibus/component/wayexpand.xml"
"$release_dir/scripts/install-xdg-systemd-dropins.sh" \
    "$unit_dir" "$config_home" "$config_dir" "$state_home" "$state_dir" "$library_dir/current/bin"
ln -sfn "$library_dir/current/desktop/wayexpand.desktop" "$application_dir/wayexpand.desktop"
if [ -f "$release_dir/io.github.cyberducttape.WayExpand.metainfo.xml" ]; then
    install -Dm644 "$release_dir/io.github.cyberducttape.WayExpand.metainfo.xml" \
        "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
fi
if [ -f "$release_dir/docs/wayexpand.1" ]; then
    install -Dm644 "$release_dir/docs/wayexpand.1" "$man_dir/wayexpand.1"
fi

icon_base="$HOME/.local/share/icons/hicolor"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    icon_src="$release_dir/icons/hicolor/$size/apps/wayexpand.png"
    if [ -f "$icon_src" ]; then
        install -Dm644 "$icon_src" "$icon_base/$size/apps/wayexpand.png"
    fi
done

if [ -e "$config_path" ] || [ -L "$config_path" ]; then
    printf '%s\n' "Keeping existing configuration: $config_path"
elif [ -f "$release_dir/expansions.toml" ]; then
    install -Dm600 "$release_dir/expansions.toml" "$config_path"
    printf '%s\n' "Installed example configuration: $config_path"
fi
broker_config="$config_dir/broker.toml"
if [ -e "$broker_config" ] || [ -L "$broker_config" ]; then
    printf '%s\n' "Keeping existing broker policy: $broker_config"
elif [ -f "$release_dir/broker.toml.example" ]; then
    install -Dm600 "$release_dir/broker.toml.example" "$broker_config"
    printf '%s\n' "Installed broker policy template: $broker_config"
fi

printf '%s\n' "Installed binaries in $bin_dir"
printf '%s\n' "Installed user units in $unit_dir"
printf '%s\n' "Installed desktop entry in $application_dir"
if [ -d "$release_dir/icons" ]; then
    printf '%s\n' "Installed application icon in $icon_base"
fi
printf '%s\n' "Next steps:"
printf '%s\n' "  export PATH=\"$bin_dir:\$PATH\""
printf '%s\n' "  systemctl --user daemon-reload"
printf '%s\n' "  wayexpand setup"
printf '%s\n' "  wayexpand status"
printf '%s\n' "  wayexpand doctor"
printf '%s\n' "  # If named actions are configured, edit $config_dir/broker.toml and enable:"
printf '%s\n' "  systemctl --user enable --now wayexpand-action-broker.service"
printf '%s\n' "  wayexpand explain-backend"
printf '%s\n' "  wayexpand edit"
printf '%s\n' "Setup never grants raw-input permissions or portal consent. If it reports"
printf '%s\n' "no safe automatic path, review doctor and the documented explicit modes."
printf '%s\n' "To remove this installation later, run scripts/uninstall-user.sh."

if command -v systemctl >/dev/null 2>&1; then
    for active_service in wayexpand-input-method.service wayexpand-evdev.service wayexpand-action-broker.service; do
        if systemctl --user is-active --quiet "$active_service" 2>/dev/null; then
            printf '%s\n' "Active service $active_service still uses its current process; restart it after this upgrade:"
            printf '%s\n' "  systemctl --user restart $active_service"
        fi
    done
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
        if [ -n "$previous_version" ]; then
            rollback_link="$library_dir/.rollback-$$"
            ln -s "$previous_version" "$rollback_link"
            mv -Tf "$rollback_link" "$library_dir/current"
        fi
        systemctl --user daemon-reload || true
        if [ "$service_was_enabled" -eq 1 ]; then
            systemctl --user restart "$service_name" || true
        else
            systemctl --user disable --now "$service_name" >/dev/null 2>&1 || true
        fi
        exit 1
    fi
    printf '%s\n' "Enabled $service_name"
fi
