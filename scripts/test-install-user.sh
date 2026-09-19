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

HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$project_dir/scripts/install-user.sh"

[ -x "$test_root/home/.local/bin/wayexpand" ]
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/bin/wayexpand-ui" ]
[ -x "$test_root/home/.local/bin/wayexpand-gui" ]
[ -f "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ -f "$test_root/home/.local/share/metainfo/io.github.itchyitchy123.WayExpand.metainfo.xml" ]
[ -f "$test_root/home/.local/share/man/man1/wayexpand.1" ]
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
