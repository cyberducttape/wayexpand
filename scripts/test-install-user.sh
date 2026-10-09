#!/bin/sh
set -eu

# Debug environment for CI troubleshooting
if [ -n "${CI:-}" ]; then
    echo "CI environment detected. System info:"
    echo "  TMPDIR=${TMPDIR:-unset}"
    echo "  GITHUB_WORKSPACE=${GITHUB_WORKSPACE:-unset}"
    echo "  HOME=$HOME"
    echo "  PWD=$PWD"
    df -h / 2>/dev/null | head -2 || true
fi

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# Ensure TMPDIR is set for mktemp; some CI runners don't set it, and mktemp
# fails silently or behaves unexpectedly without it.
: "${TMPDIR:=/tmp}"
export TMPDIR
if ! test_root=$(mktemp -d "${TMPDIR}/wayexpand-install-test.XXXXXX" 2>&1); then
    echo "error: mktemp failed: $test_root (TMPDIR=$TMPDIR)" >&2
    exit 1
fi
trap 'rm -rf "$test_root"' EXIT INT TERM
# Keep Cargo's dependency cache from the invoking environment. The test
# deliberately changes HOME to isolate user-installed files; without this,
# Cargo also looks in a fresh empty home and clean CI runners fail before the
# installer can be exercised.
cargo_home=${CARGO_HOME:-"$HOME/.cargo"}
export CARGO_HOME="$cargo_home"
# Reuse the target directory populated by earlier CI checks when the caller
# did not provide one. This keeps the installer test focused on installation
# behavior instead of recompiling the entire GUI dependency graph from zero.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-"$project_dir/target"}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

mkdir -p "$test_root/home" "$test_root/config"

# Invalid enablement options must fail before Cargo is invoked or any files
# are created under the selected home.
if HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$project_dir/scripts/install-user.sh" --enable >/dev/null 2>&1; then
    echo 'installer accepted --enable without --service' >&2
    exit 1
fi
[ ! -e "$test_root/home/.local" ]

HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$project_dir/scripts/install-user.sh"

[ -x "$test_root/home/.local/bin/wayexpand" ]
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/bin/wayexpand-action-broker" ]
[ -x "$test_root/home/.local/bin/wayexpand-ui" ]
[ -x "$test_root/home/.local/bin/wayexpand-gui" ]
[ -x "$test_root/home/.local/bin/wayexpand-ibus" ]
[ -L "$test_root/home/.local/lib/wayexpand/current" ]
[ -x "$test_root/home/.local/lib/wayexpand/current/bin/wayexpand-daemon" ]
first_build=$(readlink "$test_root/home/.local/lib/wayexpand/current")

# A second source identity with the same Cargo package version must activate
# its own staged files rather than silently reusing the first build directory.
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
WAYEXPAND_BUILD_SHA=install-test-next \
"$project_dir/scripts/install-user.sh"
second_build=$(readlink "$test_root/home/.local/lib/wayexpand/current")
[ "$second_build" != "$first_build" ]
[ -x "$test_root/home/.local/lib/wayexpand/$first_build/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/lib/wayexpand/$second_build/bin/wayexpand-daemon" ]
[ -f "$test_root/home/.local/share/ibus/component/wayexpand.xml" ]
[ -f "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ -f "$test_root/home/.local/share/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml" ]
[ -f "$test_root/home/.local/share/man/man1/wayexpand.1" ]
[ -f "$test_root/config/systemd/user/wayexpand-input-method.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-evdev.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-action-broker.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-input-method.service.d/10-xdg-paths.conf" ]
[ -f "$test_root/config/systemd/user/wayexpand-evdev.service.d/10-xdg-paths.conf" ]
[ -f "$test_root/config/systemd/user/wayexpand-action-broker.service.d/10-xdg-paths.conf" ]
[ -d "$test_root/home/.local/state/wayexpand" ]
[ "$(stat -c '%a' "$test_root/home/.local/state/wayexpand")" = 700 ]
[ -f "$test_root/config/wayexpand/broker.toml" ]
[ ! -e "$test_root/config/systemd/user/wayexpand.service" ]

custom_state_home="$test_root/custom-state"
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
XDG_STATE_HOME="$custom_state_home" \
"$project_dir/scripts/install-user.sh"
[ -d "$custom_state_home/wayexpand" ]
[ "$(stat -c '%a' "$custom_state_home/wayexpand")" = 700 ]
grep -F "ReadWritePaths=%t \"$test_root/config/wayexpand\" \"$custom_state_home/wayexpand\"" \
    "$test_root/config/systemd/user/wayexpand-action-broker.service.d/10-xdg-paths.conf"
grep -F "WAYEXPAND_PORTAL_TOKEN_PATH=$test_root/config/wayexpand/libei-portal-token" \
    "$test_root/config/systemd/user/wayexpand-input-method.service.d/10-xdg-paths.conf"
grep -F -- "--config \"$test_root/config/wayexpand/broker.toml\"" \
    "$test_root/config/systemd/user/wayexpand-action-broker.service.d/10-xdg-paths.conf"

if HOME="$test_root/home" XDG_CONFIG_HOME=relative \
    "$project_dir/scripts/install-user.sh" >/dev/null 2>&1; then
    echo "installer accepted a relative XDG_CONFIG_HOME" >&2
    exit 1
fi
if HOME="$test_root/home" XDG_CONFIG_HOME=relative \
    "$project_dir/scripts/uninstall-user.sh" >/dev/null 2>&1; then
    echo "uninstaller accepted a relative XDG_CONFIG_HOME" >&2
    exit 1
fi

config_path="$test_root/config/wayexpand/expansions.toml"
custom_config=$(mktemp "$test_root/custom-config.XXXXXX")
printf '%s\n' \
    '[[expansion]]' \
    'trigger = ":preserve"' \
    'replacement = "custom"' >"$custom_config"
mv "$custom_config" "$config_path"
cp "$config_path" "$test_root/expected-config.toml"

HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$project_dir/scripts/install-user.sh"

cmp -s "$config_path" "$test_root/expected-config.toml"

# The installer-generated service configuration remains valid for custom XDG
# roots containing spaces, and systemd specifiers in filesystem paths are escaped.
space_config_home="$test_root/config with spaces%"
space_state_home="$test_root/state with spaces%"
space_config_systemd="$test_root/config with spaces%%"
HOME="$test_root/home" XDG_CONFIG_HOME="$space_config_home" \
XDG_STATE_HOME="$space_state_home" "$project_dir/scripts/install-user.sh"
grep -F "Environment=\"XDG_CONFIG_HOME=$space_config_systemd\"" \
    "$space_config_home/systemd/user/wayexpand-evdev.service.d/10-xdg-paths.conf"
grep -F 'config with spaces%%/wayexpand/broker.toml' \
    "$space_config_home/systemd/user/wayexpand-action-broker.service.d/10-xdg-paths.conf"

# Enabled upgrades validate before replacing installed files. Keep a sentinel
# binary when the existing library is rejected by the preflight.
printf '%s\n' 'invalid existing library' >"$config_path"
printf '%s\n' 'installed-daemon-sentinel' >"$test_root/home/.local/bin/wayexpand-daemon"
stub_bin="$test_root/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 0755 "$stub_bin/systemctl"
if PATH="$stub_bin:$PATH" \
    HOME="$test_root/home" \
    XDG_CONFIG_HOME="$test_root/config" \
    "$project_dir/scripts/install-user.sh" --enable --service=wayexpand-input-method.service \
    >/dev/null 2>&1; then
    printf '%s\n' 'user installer accepted an invalid configuration during upgrade' >&2
    exit 1
fi
grep -Fx 'installed-daemon-sentinel' "$test_root/home/.local/bin/wayexpand-daemon" >/dev/null

printf '%s\n' "user installer test passed"
