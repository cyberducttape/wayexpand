#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-soak-isolation.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
runtime_dir="$test_root/runtime"
config_path="$runtime_dir/expansions.toml"
mkdir -m 0700 "$runtime_dir"
mkdir -m 0700 "$runtime_dir/home"

# Inject desktop-session handles that would escape the private XDG runtime if
# the soak ever stopped using the explicit child environment again.
export DBUS_SESSION_BUS_ADDRESS='unix:path=/tmp/should-not-be-used'
export WAYLAND_DISPLAY='wayland-should-not-be-used'
export DISPLAY=':should-not-be-used'
export XDG_CURRENT_DESKTOP='KDE'
export XDG_SESSION_TYPE='wayland'
. "$project_dir/scripts/soak-isolation.sh"

actual="$test_root/actual.env"
expected="$test_root/expected.env"
run_isolated "$runtime_dir" "$config_path" env | sort >"$actual"
env -i \
    PATH="$PATH" \
    HOME="$runtime_dir/home" \
    TMPDIR="$runtime_dir" \
    XDG_RUNTIME_DIR="$runtime_dir" \
    WAYEXPAND_CONFIG="$config_path" \
    RUST_LOG=warn \
    env | sort >"$expected"
cmp "$expected" "$actual"
printf '%s\n' 'daemon soak environment isolation passed'
