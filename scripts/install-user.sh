#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
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
cargo_bin=$(command -v cargo 2>/dev/null || true)
if [ -z "$cargo_bin" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
fi
if [ -z "$cargo_bin" ]; then
    printf '%s\n' "error: Cargo was not found" >&2
    printf '%s\n' "install Rust with rustup, restart your shell, and rerun this script:" >&2
    printf '%s\n' "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    printf '%s\n' "  . \"\$HOME/.cargo/env\"" >&2
    exit 127
fi
bin_dir="$HOME/.local/bin"
library_dir="$HOME/.local/lib/wayexpand"
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$project_dir/Cargo.toml" | head -n 1)
if [ -z "$version" ]; then
    printf '%s\n' 'error: could not determine the WayExpand version from Cargo.toml' >&2
    exit 1
fi
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
state_dir="$state_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$HOME/.local/share/applications"
ibus_component_dir="$HOME/.local/share/ibus/component"
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
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac

config_path="$config_dir/expansions.toml"

printf '%s\n' "Building WayExpand release binaries..."
CARGO_TARGET_DIR="$target_dir" "$cargo_bin" build --locked --release \
    --manifest-path "$project_dir/Cargo.toml" \
    -p wayexpand-daemon \
    -p action-broker \
    -p wayexpand \
    -p wayexpand-ui \
    -p wayexpand-gui \
    -p wayexpand-backend-ibus

if [ "$enable_service" -eq 1 ]; then
    validation_path=
    if [ -e "$config_path" ] || [ -L "$config_path" ]; then
        validation_path="$config_path"
        printf '%s\n' "Validating existing configuration before upgrade: $config_path"
    elif [ -f "$project_dir/expansions.toml" ]; then
        validation_path="$project_dir/expansions.toml"
        printf '%s\n' "Validating bundled configuration before installation: $validation_path"
    else
        printf '%s\n' 'error: --enable requires an existing configuration or bundled expansions.toml' >&2
        exit 1
    fi
    # Validate before replacing any installed files. An upgrade must not leave
    # a mixed-version installation behind when the active library is invalid.
    "$target_dir/release/wayexpand" validate "$validation_path"
fi

install -d -m 0700 "$state_dir"
chmod 0700 "$state_dir"

install -d -m 0755 "$library_dir"
stage_dir="$library_dir/.staging-$version-$$"
install -d -m 0755 "$stage_dir/bin" "$stage_dir/systemd" "$stage_dir/desktop" "$stage_dir/ibus"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    install -m 0755 "$target_dir/release/$binary" "$stage_dir/bin/$binary"
done
install -m 0644 "$project_dir"/systemd/*.service "$stage_dir/systemd/"
install -m 0644 "$project_dir/desktop/wayexpand.desktop" "$stage_dir/desktop/wayexpand.desktop"
install -m 0644 "$project_dir/desktop/wayexpand-ibus.xml" "$stage_dir/ibus/wayexpand.xml"
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
install -d -m 0755 "$unit_dir" "$ibus_component_dir" "$application_dir"
for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-action-broker.service; do
    ln -sfn "$library_dir/current/systemd/$unit" "$unit_dir/$unit"
done
ln -sfn "$library_dir/current/ibus/wayexpand.xml" "$ibus_component_dir/wayexpand.xml"
"$project_dir/scripts/install-xdg-systemd-dropins.sh" \
    "$unit_dir" "$config_home" "$config_dir" "$state_home" "$state_dir" "$library_dir/current/bin"
ln -sfn "$library_dir/current/desktop/wayexpand.desktop" "$application_dir/wayexpand.desktop"
install -Dm644 "$project_dir/io.github.cyberducttape.WayExpand.metainfo.xml" \
    "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
install -Dm644 "$project_dir/docs/wayexpand.1" "$man_dir/wayexpand.1"

icon_base="$HOME/.local/share/icons/hicolor"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    install -Dm644 "$project_dir/assets/icon/hicolor/$size/apps/wayexpand.png" \
        "$icon_base/$size/apps/wayexpand.png"
done

if [ -e "$config_path" ] || [ -L "$config_path" ]; then
    printf '%s\n' "Keeping existing configuration: $config_path"
else
    install -Dm600 "$project_dir/expansions.toml" "$config_path"
    printf '%s\n' "Installed example configuration: $config_path"
fi
broker_config="$config_dir/broker.toml"
if [ -e "$broker_config" ] || [ -L "$broker_config" ]; then
    printf '%s\n' "Keeping existing broker policy: $broker_config"
else
    install -Dm600 "$project_dir/broker.toml.example" "$broker_config"
    printf '%s\n' "Installed broker policy template: $broker_config"
fi

printf '%s\n' "Installed binaries in $bin_dir"
printf '%s\n' "Installed user units in $unit_dir"
printf '%s\n' "Installed desktop entry in $application_dir"
printf '%s\n' "Installed IBus component in $ibus_component_dir"
printf '%s\n' "Installed application icon in $icon_base"
printf '%s\n' ""
printf '%s\n' "=== NEXT STEPS ==="
printf '%s\n' ""
printf '%s\n' "1. Add to PATH:"
printf '%s\n' "   export PATH=\"$bin_dir:\$PATH\""
printf '%s\n' ""
printf '%s\n' "2. Reload systemd:"
printf '%s\n' "   systemctl --user daemon-reload"
printf '%s\n' "   # If named actions are configured, edit $broker_config and enable:"
printf '%s\n' "   systemctl --user enable --now wayexpand-action-broker.service"
if command -v systemctl >/dev/null 2>&1 && systemctl --user is-active --quiet wayexpand-action-broker.service 2>/dev/null; then
    printf '%s\n' "Active broker service still uses its current process; restart it after this upgrade:"
    printf '%s\n' "  systemctl --user restart wayexpand-action-broker.service"
fi
printf '%s\n' ""
printf '%s\n' "3. Check which backend your compositor supports:"
printf '%s\n' "   wayexpand doctor"
printf '%s\n' ""
printf '%s\n' "4. Run guided setup (it selects a safe detected compatibility mode):"
printf '%s\n' "   wayexpand setup"
printf '%s\n' "   wayexpand status"
printf '%s\n' "   wayexpand doctor"
printf '%s\n' "   (setup never grants raw-input permissions or portal consent)"
printf '%s\n' ""
printf '%s\n' "5. Edit snippets:"
printf '%s\n' "   wayexpand edit"
printf '%s\n' ""
printf '%s\n' "The stdin test harness is not installed as a user service."

if command -v systemctl >/dev/null 2>&1; then
    for active_service in wayexpand-input-method.service wayexpand-evdev.service; do
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
