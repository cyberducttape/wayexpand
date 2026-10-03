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

# The source archive is used by release packaging and by PKGBUILD's checksum.
# Keep gzip and archive entry timestamps fixed so rebuilding the same tree is
# byte-for-byte reproducible.
git archive --format=tar --mtime='1970-01-01 00:00:00' \
    --prefix=wayexpand-test/ "HEAD^{tree}" | gzip -n > "$test_root/source-a.tar.gz"
sleep 1
git archive --format=tar --mtime='1970-01-01 00:00:00' \
    --prefix=wayexpand-test/ "HEAD^{tree}" | gzip -n > "$test_root/source-b.tar.gz"
cmp "$test_root/source-a.tar.gz" "$test_root/source-b.tar.gz"
if tar -tzf "$test_root/source-a.tar.gz" | grep -E '(^|/)\.git(/|$)' >/dev/null; then
    printf '%s\n' 'release source archive unexpectedly contains Git metadata' >&2
    exit 1
fi

CARGO_TARGET_DIR="$target_dir" cargo build --locked --release \
    --manifest-path "$project_dir/Cargo.toml" \
    -p wayexpand -p wayexpand-daemon -p action-broker -p wayexpand-ui -p wayexpand-gui

bin_dir="$test_root/home/.local/bin"
config_dir="$test_root/home/.config/wayexpand"
runtime_dir="$test_root/runtime"
mkdir -p "$bin_dir" "$config_dir" "$runtime_dir"
# mkdir honors the umask, and a default of 002 (Debian/Ubuntu
# user-private-group setups) leaves these group-writable at 0775, which the
# configuration directory trust check correctly refuses to load from. Assert
# the mode the fixture means instead of inheriting the developer's shell.
chmod -R go-w "$test_root"
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
doctor_json=$(env -u XDG_RUNTIME_DIR wayexpand doctor --json "$config_path" 2>/dev/null || true)
printf '%s' "$doctor_json" | grep -F "\"config\":{\"error\":null,\"path\":\"$config_path\",\"valid\":true}" >/dev/null
printf '%s' "$doctor_json" | grep -F '"policy":{"exists":' >/dev/null

[ "$(stat -c '%a' "$config_path")" = 600 ]

# A staged package must also work when it does not ship an example into the
# user's home. `setup` is the explicit first-run initializer and must create a
# private config before attempting to start the selected user service.
fresh_root=$(mktemp -d "$test_root/fresh-first-run.XXXXXX")
fresh_home="$fresh_root/home"
fresh_config_home="$fresh_root/config"
fresh_bin="$fresh_root/bin"
fresh_systemctl_log="$fresh_root/systemctl.log"
mkdir -p "$fresh_home" "$fresh_config_home" "$fresh_bin"
chmod 0700 "$fresh_home" "$fresh_config_home"
cat >"$fresh_bin/systemctl" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"${WAYEXPAND_TEST_SYSTEMCTL_LOG:?}"
exit 0
EOF
chmod 0755 "$fresh_bin/systemctl"
install -m 0755 "$target_dir/release/wayexpand" "$fresh_bin/wayexpand"
install -m 0755 "$target_dir/release/wayexpand-daemon" "$fresh_bin/wayexpand-daemon"
: >"$fresh_systemctl_log"
HOME="$fresh_home" \
XDG_CONFIG_HOME="$fresh_config_home" \
WAYEXPAND_TEST_SYSTEMCTL_LOG="$fresh_systemctl_log" \
PATH="$fresh_bin:/usr/bin:/bin" \
    wayexpand setup --backend=input-method --yes >/dev/null
fresh_config="$fresh_config_home/wayexpand/expansions.toml"
[ -f "$fresh_config" ]
[ "$(stat -c '%a' "$fresh_config")" = 600 ]
HOME="$fresh_home" \
XDG_CONFIG_HOME="$fresh_config_home" \
PATH="$fresh_bin:/usr/bin:/bin" \
    wayexpand validate >/dev/null
grep -F -- '--user enable --now wayexpand-input-method.service' "$fresh_systemctl_log" >/dev/null

printf '%s\n' "release smoke test passed"
