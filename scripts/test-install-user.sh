#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-install-test.XXXXXX")
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

HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$project_dir/scripts/install-user.sh"

[ -x "$test_root/home/.local/bin/wayexpand" ]
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/bin/wayexpand-ui" ]
[ -x "$test_root/home/.local/bin/wayexpand-gui" ]
[ -f "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ -f "$test_root/config/systemd/user/wayexpand.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-input-method.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-evdev.service" ]

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
printf '%s\n' "user installer test passed"
