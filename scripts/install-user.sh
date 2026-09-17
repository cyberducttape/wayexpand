#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
enable_service=0
service_name=wayexpand-input-method.service
for argument in "$@"; do
    case "$argument" in
        --enable)
            enable_service=1
            ;;
        --service=wayexpand.service)
            service_name=wayexpand.service
            ;;
        --service=wayexpand-input-method.service)
            service_name=wayexpand-input-method.service
            ;;
        --service=wayexpand-evdev.service)
            service_name=wayexpand-evdev.service
            ;;
        --help|-h)
            printf '%s\n' "usage: $0 [--enable] [--service=wayexpand.service|wayexpand-input-method.service|wayexpand-evdev.service]"
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
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$HOME/.local/share/applications"
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac

printf '%s\n' "Building WayExpand release binaries..."
export RUSTUP_TOOLCHAIN=stable
CARGO_TARGET_DIR="$target_dir" "$cargo_bin" build --locked --release \
    --manifest-path "$project_dir/Cargo.toml" \
    -p wayexpand-daemon \
    -p wayexpand \
    -p wayexpand-ui \
    -p wayexpand-gui

install -Dm755 "$target_dir/release/wayexpand-daemon" \
    "$bin_dir/wayexpand-daemon"
install -Dm755 "$target_dir/release/wayexpand" \
    "$bin_dir/wayexpand"
install -Dm755 "$target_dir/release/wayexpand-ui" \
    "$bin_dir/wayexpand-ui"
install -Dm755 "$target_dir/release/wayexpand-gui" \
    "$bin_dir/wayexpand-gui"
install -Dm644 "$project_dir/systemd/wayexpand.service" \
    "$unit_dir/wayexpand.service"
install -Dm644 "$project_dir/systemd/wayexpand-input-method.service" \
    "$unit_dir/wayexpand-input-method.service"
install -Dm644 "$project_dir/systemd/wayexpand-evdev.service" \
    "$unit_dir/wayexpand-evdev.service"
install -Dm644 "$project_dir/desktop/wayexpand.desktop" \
    "$application_dir/wayexpand.desktop"

config_path="$config_dir/expansions.toml"
if [ -e "$config_path" ] || [ -L "$config_path" ]; then
    printf '%s\n' "Keeping existing configuration: $config_path"
else
    install -Dm600 "$project_dir/expansions.toml" "$config_path"
    printf '%s\n' "Installed example configuration: $config_path"
fi

printf '%s\n' "Installed binaries in $bin_dir"
printf '%s\n' "Installed user units in $unit_dir"
printf '%s\n' "Installed desktop entry in $application_dir"
printf '%s\n' "Next steps:"
printf '%s\n' "  export PATH=\"$bin_dir:\$PATH\""
printf '%s\n' "  systemctl --user daemon-reload"
printf '%s\n' "  wayexpand doctor"
printf '%s\n' "  systemctl --user enable --now wayexpand-input-method.service"
printf '%s\n' "  wayexpand-gui"
printf '%s\n' "If \`doctor\` reports no input-method-v2/virtual-keyboard support (for"
printf '%s\n' "example on KWin/KDE Plasma), read docs/SECURITY.md and consider:"
printf '%s\n' "  sudo ./scripts/install-evdev-permissions.sh --dry-run"
printf '%s\n' "  systemctl --user enable --now wayexpand-evdev.service"

if [ "$enable_service" -eq 1 ]; then
    "$bin_dir/wayexpand" validate "$config_path"
    if ! command -v systemctl >/dev/null 2>&1; then
        printf '%s\n' "error: systemctl is required for --enable" >&2
        exit 127
    fi
    printf '%s\n' "Enabling user service: $service_name"
    systemctl --user daemon-reload
    systemctl --user enable --now "$service_name"
    printf '%s\n' "Enabled $service_name"
fi
