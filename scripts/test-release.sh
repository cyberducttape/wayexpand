#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-release-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM

CARGO_TARGET_DIR="$target_dir" cargo build --locked --release \
    --manifest-path "$project_dir/Cargo.toml" \
    -p wayexpand -p wayexpand-daemon -p wayexpand-ui -p wayexpand-gui

bin_dir="$test_root/home/.local/bin"
config_dir="$test_root/home/.config/wayexpand"
runtime_dir="$test_root/runtime"
mkdir -p "$bin_dir" "$config_dir" "$runtime_dir"
install -m 0755 "$target_dir/release/wayexpand" "$bin_dir/"
install -m 0755 "$target_dir/release/wayexpand-daemon" "$bin_dir/"
install -m 0755 "$target_dir/release/wayexpand-ui" "$bin_dir/"
install -m 0755 "$target_dir/release/wayexpand-gui" "$bin_dir/"
install -m 0600 "$project_dir/expansions.toml" "$config_dir/expansions.toml"

export HOME="$test_root/home"
export PATH="$bin_dir:/usr/bin:/bin"
config_path="$config_dir/expansions.toml"

wayexpand validate "$config_path" >/dev/null
wayexpand test ';;hello' "$config_path" | grep -Fx 'Hello from Wayland!'
wayexpand test-hotkey Ctrl+Alt+M --json "$config_path" | grep -F '"matched":false'
env -u XDG_RUNTIME_DIR wayexpand doctor --json "$config_path" | grep -F '"healthy":true'

[ "$(stat -c '%a' "$config_path")" = 600 ]
printf '%s\n' "release smoke test passed"
