#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-release-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM

release_dir="$test_root/wayexpand-0.0.0-linux-x86_64"
mkdir -p "$release_dir/bin" "$release_dir/systemd" "$release_dir/desktop" "$release_dir/docs" "$release_dir/scripts" "$release_dir/udev" "$release_dir/ibus/component"
for binary in wayexpand-daemon wayexpand wayexpand-ui wayexpand-gui wayexpand-ibus; do
    printf '#!/bin/sh\nexit 0\n' >"$release_dir/bin/$binary"
    chmod 0755 "$release_dir/bin/$binary"
done
install -m 0644 "$project_dir/systemd/wayexpand.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/systemd/wayexpand-input-method.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/systemd/wayexpand-evdev.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/desktop/wayexpand.desktop" "$release_dir/desktop/"
install -m 0644 "$project_dir/desktop/wayexpand-ibus.xml" "$release_dir/ibus/component/"
install -m 0644 "$project_dir/expansions.toml" "$release_dir/expansions.toml"
install -m 0755 "$project_dir/scripts/install-release.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/uninstall-user.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/install-evdev-permissions.sh" "$release_dir/scripts/"
install -m 0644 "$project_dir/udev/71-wayexpand-evdev.rules" "$release_dir/udev/"

# install-evdev-permissions.sh must work from this release layout too (it
# looks for udev/71-wayexpand-evdev.rules next to its own scripts/ dir).
"$release_dir/scripts/install-evdev-permissions.sh" --dry-run >/dev/null

mkdir -p "$test_root/home" "$test_root/config"
install -m 0644 "$project_dir/io.github.itchyitchy123.WayExpand.metainfo.xml" \
    "$release_dir/io.github.itchyitchy123.WayExpand.metainfo.xml"
install -m 0644 "$project_dir/docs/wayexpand.1" "$release_dir/docs/wayexpand.1"

# Stub out systemctl so this test never touches the invoking user's real
# systemd session, no matter what HOME/XDG_CONFIG_HOME are set to (systemctl
# talks to the session bus, which is independent of those variables).
stub_bin="$test_root/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 0755 "$stub_bin/systemctl"

PATH="$stub_bin:$PATH" \
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$release_dir/scripts/install-release.sh"

[ -x "$test_root/home/.local/bin/wayexpand" ]
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/bin/wayexpand-ui" ]
[ -x "$test_root/home/.local/bin/wayexpand-gui" ]
[ -x "$test_root/home/.local/bin/wayexpand-ibus" ]
[ -f "$test_root/home/.local/share/ibus/component/wayexpand.xml" ]
[ -f "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ -f "$test_root/home/.local/share/metainfo/io.github.itchyitchy123.WayExpand.metainfo.xml" ]
[ -f "$test_root/home/.local/share/man/man1/wayexpand.1" ]
[ -f "$test_root/config/systemd/user/wayexpand.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-input-method.service" ]
[ -f "$test_root/config/systemd/user/wayexpand-evdev.service" ]
[ -f "$test_root/config/wayexpand/expansions.toml" ]

PATH="$stub_bin:$PATH" \
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$release_dir/scripts/uninstall-user.sh"

[ ! -e "$test_root/home/.local/bin/wayexpand" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-daemon" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-ui" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-gui" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-ibus" ]
[ ! -e "$test_root/home/.local/share/ibus/component/wayexpand.xml" ]
[ ! -e "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ ! -e "$test_root/home/.local/share/metainfo/io.github.itchyitchy123.WayExpand.metainfo.xml" ]
[ ! -e "$test_root/home/.local/share/man/man1/wayexpand.1" ]
[ ! -e "$test_root/config/systemd/user/wayexpand.service" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-input-method.service" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-evdev.service" ]
[ -f "$test_root/config/wayexpand/expansions.toml" ]

printf '%s\n' "release install/uninstall test passed"
