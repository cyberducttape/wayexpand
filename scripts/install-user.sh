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
if [ "$(id -u)" -eq 0 ]; then
    if [ -n "${SUDO_USER:-}" ]; then
        printf '%s\n' "error: do not run the user installer with sudo; run it as $SUDO_USER" >&2
    else
        printf '%s\n' "error: this is a per-user installer; run it as your desktop user, not root" >&2
    fi
    exit 1
fi
printf '%s\n' "Desktop compatibility (likely route; verify in your own applications):"
printf '%s\n' "  GNOME Shell 46, 48, 50, 51: IBus + WayExpand extension (headless-tested; no real-session certification)"
printf '%s\n' "  GNOME Shell 49: extension support undeclared; German/French layout switching unverified in a real session"
printf '%s\n' "  Other GNOME Shell majors: extension support undeclared; compatibility unknown"
printf '%s\n' "  KDE Plasma / other Wayland desktops: setup checks available routes; application behavior is unverified"
printf '%s\n' "  If no automatic route is verified, Quick Picker can copy snippets for manual paste."
case ":${XDG_CURRENT_DESKTOP:-}:${XDG_SESSION_DESKTOP:-}:" in
    *:GNOME:*|*:gnome:*)
        if command -v gnome-shell >/dev/null 2>&1; then
            detected_gnome_shell_version=$(gnome-shell --version 2>/dev/null || true)
            if [ -n "$detected_gnome_shell_version" ]; then
                detected_gnome_shell_major=$(printf '%s\n' "$detected_gnome_shell_version" | awk '{for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+([.][0-9]+)*$/) {split($i, parts, "."); print parts[1]; exit}}')
                printf '%s\n' "Detected GNOME Shell: $detected_gnome_shell_version"
                case "$detected_gnome_shell_major" in
                    46|48|50|51)
                        printf '%s\n' "  This GNOME major has headless integration coverage; real-session insertion is not certified."
                        ;;
                    49)
                        printf '%s\n' "  GNOME 49 extension support is undeclared; German/French layout switching is unverified in a real session."
                        ;;
                    '')
                        printf '%s\n' "  GNOME Shell version could not be parsed; compare it with the compatibility table above."
                        ;;
                    *)
                        printf '%s\n' "  GNOME Shell $detected_gnome_shell_major is not a declared extension target; compatibility is unknown."
                        ;;
                esac
            else
                printf '%s\n' "GNOME Shell is installed but its version could not be read; compare it with the compatibility table above."
            fi
        else
            printf '%s\n' "Check GNOME version with: gnome-shell --version"
        fi
        ;;
esac
printf '%s\n' ""
cargo_bin=$(command -v cargo 2>/dev/null || true)
if [ -z "$cargo_bin" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
fi
if [ -z "$cargo_bin" ]; then
    printf '%s\n' "error: Cargo was not found; building WayExpand from source needs Rust" >&2
    printf '%s\n' "install Rust with rustup (https://rustup.rs), open a new terminal, and run this script again:" >&2
    printf '%s\n' "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    printf '%s\n' "or install a published package or release archive, which needs no Rust toolchain:" >&2
    printf '%s\n' "  https://github.com/cyberducttape/wayexpand/releases" >&2
    exit 127
fi

# Distribution packages of Rust are often older than the project's minimum
# (Ubuntu 24.04 ships 1.75). Say so before a long build fails on it.
required_rust=$(sed -n 's/^rust-version = "\([0-9.]*\)"/\1/p' "$project_dir/Cargo.toml" | head -n1)
cargo_version=$(cd "$project_dir" && "$cargo_bin" --version 2>/dev/null | awk '{print $2}' || true)
if [ -n "$required_rust" ] && [ -n "$cargo_version" ]; then
    oldest=$(printf '%s\n%s\n' "$required_rust" "${cargo_version%%-*}" | sort -V | head -n1)
    if [ "$oldest" != "$required_rust" ]; then
        rustup_cargo="$HOME/.cargo/bin/cargo"
        rustup_version=
        if [ -x "$rustup_cargo" ] && [ "$cargo_bin" != "$rustup_cargo" ]; then
            rustup_version=$(cd "$project_dir" && "$rustup_cargo" --version 2>/dev/null | awk '{print $2}' || true)
            if [ -n "$rustup_version" ]; then
                rustup_oldest=$(printf '%s\n%s\n' "$required_rust" "${rustup_version%%-*}" | sort -V | head -n1)
                if [ "$rustup_oldest" = "$required_rust" ]; then
                    printf '%s\n' "Using rustup Cargo at $rustup_cargo ($rustup_version); PATH Cargo at $cargo_bin ($cargo_version) was too old."
                    cargo_bin=$rustup_cargo
                    cargo_version=$rustup_version
                    oldest=$required_rust
                fi
            fi
        fi
        if [ "$oldest" != "$required_rust" ]; then
            printf '%s\n' "error: Rust $required_rust or newer is required; $cargo_bin is $cargo_version" >&2
            if [ -n "$rustup_version" ]; then
                printf '%s\n' "rustup Cargo at $rustup_cargo is $rustup_version, which is also too old" >&2
            else
                printf '%s\n' "install a current toolchain with rustup (https://rustup.rs); this checkout then selects the right version itself" >&2
            fi
            exit 1
        fi
    fi
fi

# Print an installation command only when the distribution family is known.
# Guessing apt on an unknown distribution sends users toward the wrong fix.
build_dependency_hint() {
    os_ids=
    if [ -r /etc/os-release ]; then
        # shellcheck source=/dev/null
        os_ids=$( . /etc/os-release && printf '%s %s' "${ID:-}" "${ID_LIKE:-}" )
    fi
    case " $os_ids " in
        *" fedora "*|*" rhel "*|*" centos "*|*" rocky "*|*" almalinux "*)
            printf '%s\n' "  sudo dnf install gcc pkgconf-pkg-config wayland-devel libxkbcommon-devel" ;;
        *" arch "*|*" manjaro "*|*" endeavouros "*)
            printf '%s\n' "  sudo pacman -S --needed base-devel wayland libxkbcommon" ;;
        *" opensuse "*|*" opensuse-leap "*|*" opensuse-tumbleweed "*|*" suse "*)
            printf '%s\n' "  sudo zypper install gcc pkg-config wayland-devel libxkbcommon-devel" ;;
        *" debian "*|*" ubuntu "*|*" linuxmint "*|*" pop "*|*" kali "*)
            printf '%s\n' "  sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev" ;;
        *" alpine "*)
            printf '%s\n' "  sudo apk add build-base pkgconf wayland-dev libxkbcommon-dev" ;;
        *)
            printf '%s\n' "  Could not identify a supported package manager for '${os_ids:-unknown distribution}'."
            printf '%s\n' "  See docs/DEVELOPMENT.md for native build dependencies, or use a published release archive:"
            printf '%s\n' "  https://github.com/cyberducttape/wayexpand/releases" ;;
    esac
}
missing_native_build_dependencies=0
c_compiler=${CC:-cc}
c_compiler=${c_compiler%%[[:space:]]*}
if [ -z "$c_compiler" ] || ! command -v "$c_compiler" >/dev/null 2>&1; then
    printf '%s\n' "error: no C compiler was found (${CC:-cc}); a source build requires native build tools" >&2
    missing_native_build_dependencies=1
fi
if ! command -v pkg-config >/dev/null 2>&1; then
    printf '%s\n' 'error: pkg-config was not found; it is needed to locate Wayland and xkbcommon headers and libraries' >&2
    missing_native_build_dependencies=1
else
    for module in wayland-client xkbcommon; do
        if ! pkg-config --exists "$module"; then
            printf '%s\n' "error: pkg-config cannot find the $module development files" >&2
            missing_native_build_dependencies=1
        fi
    done
fi
if [ "$missing_native_build_dependencies" -ne 0 ]; then
    printf '%s\n' 'Install the native build dependencies, then rerun this script:' >&2
    build_dependency_hint >&2
    exit 1
fi
printf '%s\n' "Developer source install: compiling the application from this checkout."
printf '%s\n' "For ordinary desktop installation, use a published .deb/.rpm or release archive; no local Rust toolchain is needed."
bin_dir="$HOME/.local/bin"
library_dir="$HOME/.local/lib/wayexpand"
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
config_dir="$config_home/wayexpand"
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
state_dir="$state_home/wayexpand"
unit_dir="$config_home/systemd/user"
application_dir="$data_home/applications"
metainfo_dir="$data_home/metainfo"
man_dir="$data_home/man/man1"
# One consent-marker location for the CLI, installers, doctor, GUI and the
# unit: systemd evaluates ConditionPathExists before any XDG environment
# applies, so the marker never follows XDG_CONFIG_HOME. The evdev unit is
# inert without it, and systemd reports a skipped condition as success, so
# refuse rather than report an enabled service that never runs; never create
# the marker here.
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
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac

config_path="$config_dir/expansions.toml"

printf '%s\n' "Building WayExpand release binaries..."
if ! CARGO_TARGET_DIR="$target_dir" "$cargo_bin" build --locked --release \
    --manifest-path "$project_dir/Cargo.toml" \
    -p wayexpand-daemon \
    -p action-broker \
    -p wayexpand \
    -p wayexpand-ui \
    -p wayexpand-gui \
    -p wayexpand-backend-ibus; then
    printf '%s\n' "error: the build failed; nothing was installed" >&2
    printf '%s\n' "If the error names missing native build dependencies, install them with:" >&2
    build_dependency_hint >&2
    printf '%s\n' "then run this script again" >&2
    exit 1
fi

# Development builds intentionally retain the package version and encode the
# source commit in the binary version. Use that complete build identity for
# the immutable install directory; package version alone would cause every
# install from a newer commit to reuse stale binaries and service assets.
build_version=$(
    "$target_dir/release/wayexpand" --version |
        sed -n 's/^wayexpand \([A-Za-z0-9.+~-][A-Za-z0-9.+~-]*\) (commit [A-Za-z0-9._-]*)$/\1/p'
)
case "$build_version" in
    ''|*[!A-Za-z0-9.+~-]*)
        printf '%s\n' 'error: could not determine a safe WayExpand build identity' >&2
        exit 1
        ;;
esac

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
stage_dir="$library_dir/.staging-$build_version-$$"
# Never leave a partial payload behind if staging fails or is interrupted.
trap 'rm -rf -- "$stage_dir"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
install -d -m 0755 "$stage_dir/bin" "$stage_dir/systemd" "$stage_dir/desktop" "$stage_dir/ibus" \
    "$stage_dir/share/wayexpand"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    install -m 0755 "$target_dir/release/$binary" "$stage_dir/bin/$binary"
done
install -m 0644 "$project_dir"/systemd/*.service "$stage_dir/systemd/"
install -m 0644 "$project_dir/desktop/wayexpand.desktop" "$stage_dir/desktop/wayexpand.desktop"
install -m 0644 "$project_dir/desktop/wayexpand-ibus.xml" "$stage_dir/ibus/wayexpand.xml"
install -m 0644 "$project_dir/installer-components.toml" "$stage_dir/share/wayexpand/installer-components.toml"
install -m 0644 "$project_dir/expansions.toml" "$stage_dir/expansions.toml"
printf '%s\n' 'source-installation' >"$stage_dir/share/wayexpand/install-origin"
payload_hash=$("$project_dir/scripts/hash-install-payload.sh" "$stage_dir")
if [ "${#payload_hash}" -ne 64 ]; then
    printf '%s\n' 'error: could not determine a valid install payload hash' >&2
    exit 1
fi
case "$payload_hash" in
    *[!0-9a-f]*|'')
        printf '%s\n' 'error: could not determine a safe install payload identity' >&2
        exit 1
        ;;
esac
version="$build_version-$payload_hash"
if [ ! -e "$library_dir/$version" ] && [ ! -L "$library_dir/$version" ]; then
    mv "$stage_dir" "$library_dir/$version"
else
    if [ -L "$library_dir/$version" ] || [ ! -d "$library_dir/$version" ]; then
        printf '%s\n' "error: existing version path is not a real directory: $library_dir/$version" >&2
        exit 1
    fi
    existing_hash=$("$project_dir/scripts/hash-install-payload.sh" "$library_dir/$version") || {
        printf '%s\n' "error: existing version directory is incomplete or unsafe: $library_dir/$version" >&2
        exit 1
    }
    if [ "$existing_hash" != "$payload_hash" ]; then
        printf '%s\n' "error: existing version directory does not match its payload identity: $library_dir/$version" >&2
        exit 1
    fi
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
if [ -n "$previous_version" ]; then
    install -d -m 0700 "$state_dir"
    rollback_tmp="$state_dir/previous-install-$$"
    (umask 077; printf '%s\n' "$previous_version" > "$rollback_tmp")
    mv -f "$rollback_tmp" "$state_dir/previous-install"
fi
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
        if [ "$version_switch_started" -eq 1 ]; then
            printf '%s\n' 'error: installation failed during or after switching to the staged version; restoring the previous version' >&2
            restore_previous_version || true
        fi
        rm -f "$next_link"
    fi
    return "$install_status"
}
trap rollback_on_install_failure EXIT
ln -s "$version" "$next_link"
# Mark the transaction before mv: if a signal or an I/O error interrupts the
# atomic rename, rollback is safe whether the old or new link is visible.
version_switch_started=1
mv -Tf "$next_link" "$library_dir/current"
install -d -m 0755 "$bin_dir"
for binary in wayexpand-daemon wayexpand-action-broker wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    ln -sfn "$library_dir/current/bin/$binary" "$bin_dir/$binary"
done
install -d -m 0755 "$unit_dir" "$application_dir"
for unit in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
    ln -sfn "$library_dir/current/systemd/$unit" "$unit_dir/$unit"
done
# ibus-daemon never reads per-user component directories, so a manifest
# there cannot register the engine. Remove the link older installers made;
# `wayexpand setup --backend ibus` installs the component where IBus reads it.
stale_ibus_component="$data_home/ibus/component/wayexpand.xml"
if [ -L "$stale_ibus_component" ]; then
    case $(readlink "$stale_ibus_component") in
        "$library_dir"/*) rm -f -- "$stale_ibus_component" ;;
    esac
fi
"$project_dir/scripts/install-xdg-systemd-dropins.sh" \
    "$unit_dir" "$config_home" "$config_dir" "$state_home" "$state_dir" "$library_dir/current/bin"
ln -sfn "$library_dir/current/desktop/wayexpand.desktop" "$application_dir/wayexpand.desktop"
install -Dm644 "$project_dir/io.github.cyberducttape.WayExpand.metainfo.xml" \
    "$metainfo_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
install -Dm644 "$project_dir/docs/wayexpand.1" "$man_dir/wayexpand.1"

icon_base="$data_home/icons/hicolor"
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
printf '%s\n' "Staged IBus component in $library_dir/current/ibus (not registered with the system IBus daemon)"
printf '%s\n' "Installed application icon in $icon_base"
printf '%s\n' ""
printf '%s\n' "=== START HERE ==="
printf '%s\n' "Open WayExpand from your desktop application menu. Its setup guide checks this session,"
printf '%s\n' "shows the next action, and verifies insertion in a real application."
printf '%s\n' "If the launcher is not visible yet, start it directly: \"$bin_dir/wayexpand-gui\""
printf '%s\n' ""
printf '%s\n' ""
printf '%s\n' "=== OPTIONAL COMMAND-LINE SETUP ==="
printf '%s\n' ""
printf '%s\n' "Add to PATH to use the short 'wayexpand' command:"
printf '%s\n' "   export PATH=\"$bin_dir:\$PATH\""
case "${SHELL##*/}" in
    bash)
        printf '%s\n' "For future Bash terminals, run once: printf '\\nexport PATH=\"$bin_dir:\$PATH\"\\n' >> \"$HOME/.bashrc\"" ;;
    zsh)
        printf '%s\n' "For future Zsh terminals, run once: printf '\\nexport PATH=\"$bin_dir:\$PATH\"\\n' >> \"$HOME/.zshrc\"" ;;
    fish)
        printf '%s\n' "For future Fish terminals, run once: fish_add_path \"$bin_dir\"" ;;
    *)
        printf '%s\n' "To keep this PATH in future terminals, see $project_dir/README.md" ;;
esac
printf '%s\n' ""
printf '%s\n' "If you use named actions, edit $broker_config and enable the action service:"
printf '%s\n' "   systemctl --user enable --now wayexpand-action-broker.service"
if [ -f "$project_dir/polkit/org.wayexpand.action.policy" ] &&
    [ ! -f /usr/share/polkit-1/actions/org.wayexpand.action.policy ]; then
    printf '%s\n' "Broker actions require administrator installation of the polkit policy:"
    printf '%s\n' "   sudo install -m 0644 \"$project_dir/polkit/org.wayexpand.action.policy\" /usr/share/polkit-1/actions/"
fi
printf '%s\n' ""
printf '%s\n' "For command-line troubleshooting, run these in order:"
printf '%s\n' "   \"$bin_dir/wayexpand\" doctor"
printf '%s\n' ""
printf '%s\n' "   \"$bin_dir/wayexpand\" setup"
printf '%s\n' "   If you choose IBus and its manifest is missing, setup explains the one-time administrator authorization before installing it."
printf '%s\n' "   For scripted IBus setup, explicitly authorize that step with: \"$bin_dir/wayexpand\" setup --backend ibus --yes --install-system-integration"
printf '%s\n' "   \"$bin_dir/wayexpand\" status"
printf '%s\n' "   \"$bin_dir/wayexpand\" doctor"
printf '%s\n' "   (setup never grants raw-input permissions or portal consent)"
printf '%s\n' ""
printf '%s\n' "To edit snippets:"
printf '%s\n' "   \"$bin_dir/wayexpand\" edit"
printf '%s\n' ""
printf '%s\n' "The stdin test harness is not installed as a user service; wayexpand-control.service provides the control socket beside IBus."

if command -v systemctl >/dev/null 2>&1; then
    active_upgrade=0
    for active_service in wayexpand-input-method.service wayexpand-evdev.service wayexpand-control.service wayexpand-action-broker.service; do
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
        restore_previous_version || true
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
