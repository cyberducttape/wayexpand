#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-release-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
# Never reach the invoking user's real systemd manager or session bus, even
# where a test leaves systemctl unstubbed: any real `systemctl --user` then
# fails instead of stopping or disabling the developer's own services.
isolated_runtime="$test_root/runtime"
mkdir -m 0700 "$isolated_runtime"
export XDG_RUNTIME_DIR="$isolated_runtime"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$isolated_runtime/bus"

release_dir="$test_root/wayexpand-0.0.0-linux-x86_64"
mkdir -p "$release_dir/bin" "$release_dir/systemd" "$release_dir/desktop" "$release_dir/docs" "$release_dir/scripts" "$release_dir/udev" "$release_dir/ibus/component"
for binary in wayexpand-daemon wayexpand wayexpand-action-broker wayexpand-ui wayexpand-gui wayexpand-ibus; do
    printf '#!/bin/sh\nexit 0\n' >"$release_dir/bin/$binary"
    chmod 0755 "$release_dir/bin/$binary"
done
install -m 0644 "$project_dir/systemd/wayexpand-input-method.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/systemd/wayexpand-evdev.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/systemd/wayexpand-action-broker.service" "$release_dir/systemd/"
install -m 0644 "$project_dir/desktop/wayexpand.desktop" "$release_dir/desktop/"
install -m 0644 "$project_dir/desktop/wayexpand-ibus.xml" "$release_dir/ibus/component/"
install -m 0644 "$project_dir/expansions.toml" "$release_dir/expansions.toml"
install -m 0600 "$project_dir/broker.toml.example" "$release_dir/broker.toml.example"
install -m 0755 "$project_dir/scripts/install-release.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/install-xdg-systemd-dropins.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/hash-install-payload.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/uninstall-user.sh" "$release_dir/scripts/"
install -m 0755 "$project_dir/scripts/install-evdev-permissions.sh" "$release_dir/scripts/"
install -m 0644 "$project_dir/udev/71-wayexpand-evdev.rules" "$release_dir/udev/"
install -m 0644 "$project_dir/udev/69-wayexpand-evdev-uaccess.rules" "$release_dir/udev/"

# install-evdev-permissions.sh must work from this release layout too (it
# looks for udev/71-wayexpand-evdev.rules next to its own scripts/ dir).
"$release_dir/scripts/install-evdev-permissions.sh" --dry-run >/dev/null

mkdir -p "$test_root/home" "$test_root/config"
install -m 0644 "$project_dir/io.github.cyberducttape.WayExpand.metainfo.xml" \
    "$release_dir/io.github.cyberducttape.WayExpand.metainfo.xml"
install -m 0644 "$project_dir/docs/wayexpand.1" "$release_dir/docs/wayexpand.1"

if HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$release_dir/scripts/install-release.sh" --enable >/dev/null 2>&1; then
    printf '%s\n' 'release installer accepted --enable without --service' >&2
    exit 1
fi
[ ! -e "$test_root/home/.local" ]

# Stub out systemctl so this test never touches the invoking user's real
# systemd session, no matter what HOME/XDG_CONFIG_HOME are set to (systemctl
# talks to the session bus, which is independent of those variables).
stub_bin="$test_root/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
if [ -n "${SYSTEMCTL_LOG:-}" ]; then
    printf '%s\n' "$*" >>"$SYSTEMCTL_LOG"
fi
case "$*" in
    "--user is-active --quiet "*) exit 0 ;;
    "--user is-enabled "*|"--user is-active "*) exit 1 ;;
esac
exit 0
EOF
chmod 0755 "$stub_bin/systemctl"
systemctl_log="$test_root/systemctl.log"
: >"$systemctl_log"

PATH="$stub_bin:$PATH" \
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$release_dir/scripts/install-release.sh"

[ -x "$test_root/home/.local/bin/wayexpand" ]
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]
[ -x "$test_root/home/.local/bin/wayexpand-action-broker" ]
[ -x "$test_root/home/.local/bin/wayexpand-ui" ]
[ -x "$test_root/home/.local/bin/wayexpand-gui" ]
[ -x "$test_root/home/.local/bin/wayexpand-ibus" ]
[ -L "$test_root/home/.local/lib/wayexpand/current" ]
[ -x "$test_root/home/.local/lib/wayexpand/current/bin/wayexpand-daemon" ]
# Install directories are named by version and payload content.
first_version=$(readlink "$test_root/home/.local/lib/wayexpand/current")
case "$first_version" in
    0.0.0-*) [ "${#first_version}" -eq $((6 + 64)) ] ;;
    *) printf '%s\n' "unexpected install directory: $first_version" >&2; exit 1 ;;
esac
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
[ ! -e "$test_root/config/systemd/user/wayexpand.service" ]
[ -f "$test_root/config/wayexpand/expansions.toml" ]
[ -f "$test_root/config/wayexpand/broker.toml" ]

# Enabling evdev without its consent marker would leave an inert unit that
# systemd reports as started.
if PATH="$stub_bin:$PATH" SYSTEMCTL_LOG="$systemctl_log" HOME="$test_root/home" \
    XDG_CONFIG_HOME="$test_root/config" \
    "$release_dir/scripts/install-release.sh" --enable --service=wayexpand-evdev.service \
    >"$test_root/evdev-no-consent.out" 2>&1; then
    printf '%s\n' 'installer enabled evdev without the consent marker' >&2
    exit 1
fi
grep -F 'evdev-sensitive-fields-ack' "$test_root/evdev-no-consent.out" >/dev/null
install -d -m 0700 "$test_root/home/.config/wayexpand"
install -m 0600 /dev/null "$test_root/home/.config/wayexpand/evdev-sensitive-fields-ack"

PATH="$stub_bin:$PATH" \
SYSTEMCTL_LOG="$systemctl_log" \
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
"$release_dir/scripts/install-release.sh" \
    --enable --service=wayexpand-evdev.service
grep -F -- '--user daemon-reload' "$systemctl_log" >/dev/null
grep -F -- '--user enable wayexpand-evdev.service' "$systemctl_log" >/dev/null
grep -F -- '--user restart wayexpand-evdev.service' "$systemctl_log" >/dev/null

# A new release whose service fails to start must atomically restore the
# previously active version.
next_release="$test_root/wayexpand-0.0.1-linux-x86_64"
cp -R "$release_dir" "$next_release"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
case "$*" in
    "--user restart "*) exit 1 ;;
    "--user is-active --quiet "*) exit 0 ;;
    *) exit 0 ;;
esac
EOF
chmod 0755 "$stub_bin/systemctl"
if PATH="$stub_bin:$PATH" HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$next_release/scripts/install-release.sh" --enable --service=wayexpand-evdev.service \
    >/dev/null 2>&1; then
    printf '%s\n' 'installer accepted a service startup failure' >&2
    exit 1
fi
[ "$(readlink "$test_root/home/.local/lib/wayexpand/current")" = "$first_version" ]

enable_failure_release="$test_root/wayexpand-0.0.2-linux-x86_64"
cp -R "$release_dir" "$enable_failure_release"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
case "$*" in
    "--user enable "*) exit 1 ;;
    *) exit 0 ;;
esac
EOF
chmod 0755 "$stub_bin/systemctl"
if PATH="$stub_bin:$PATH" HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$enable_failure_release/scripts/install-release.sh" --enable --service=wayexpand-evdev.service \
    >/dev/null 2>&1; then
    printf '%s\n' 'installer accepted a systemctl enable failure' >&2
    exit 1
fi
[ "$(readlink "$test_root/home/.local/lib/wayexpand/current")" = "$first_version" ]

# A corrected archive with the same version string must not reuse the
# binaries installed from the earlier one.
corrected_release="$test_root/corrected/wayexpand-0.0.0-linux-x86_64"
mkdir -p "$test_root/corrected"
cp -R "$release_dir" "$corrected_release"
printf '#!/bin/sh\necho corrected\n' >"$corrected_release/bin/wayexpand-daemon"
printf '#!/bin/sh\nexit 0\n' >"$stub_bin/systemctl"
chmod 0755 "$stub_bin/systemctl"
PATH="$stub_bin:$PATH" HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$corrected_release/scripts/install-release.sh" >/dev/null
corrected_version=$(readlink "$test_root/home/.local/lib/wayexpand/current")
[ "$corrected_version" != "$first_version" ]
grep -Fx 'echo corrected' "$test_root/home/.local/lib/wayexpand/current/bin/wayexpand-daemon" >/dev/null
# Reinstalling identical content reuses its verified directory.
PATH="$stub_bin:$PATH" HOME="$test_root/home" XDG_CONFIG_HOME="$test_root/config" \
    "$corrected_release/scripts/install-release.sh" >/dev/null
[ "$(readlink "$test_root/home/.local/lib/wayexpand/current")" = "$corrected_version" ]

cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
if [ -n "${SYSTEMCTL_LOG:-}" ]; then
    printf '%s\n' "$*" >>"$SYSTEMCTL_LOG"
fi
case "$*" in
    "--user is-enabled "*|"--user is-active "*) exit 1 ;;
esac
exit 0
EOF
chmod 0755 "$stub_bin/systemctl"

touch "$test_root/71-wayexpand-evdev.rules" "$test_root/69-wayexpand-evdev-uaccess.rules"
PATH="$stub_bin:$PATH" \
HOME="$test_root/home" \
XDG_CONFIG_HOME="$test_root/config" \
WAYEXPAND_UNINSTALL_GROUPS="wheel input" \
WAYEXPAND_EVDEV_RULE_DEST="$test_root/71-wayexpand-evdev.rules" \
WAYEXPAND_EVDEV_UACCESS_RULE_DEST="$test_root/69-wayexpand-evdev-uaccess.rules" \
"$release_dir/scripts/uninstall-user.sh" >"$test_root/uninstall.out"

grep -F "WayExpand user files were removed." "$test_root/uninstall.out" >/dev/null
grep -F "WARNING: raw-input privileges are still configured:" "$test_root/uninstall.out" >/dev/null
grep -F "is a member of the input group" "$test_root/uninstall.out" >/dev/null
grep -F "WayExpand input-group udev rule is installed" "$test_root/uninstall.out" >/dev/null
grep -F "WayExpand active-seat udev rule is installed" "$test_root/uninstall.out" >/dev/null
grep -F "sudo $release_dir/scripts/install-evdev-permissions.sh --uninstall" "$test_root/uninstall.out" >/dev/null

[ ! -e "$test_root/home/.local/bin/wayexpand" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-daemon" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-ui" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-gui" ]
[ ! -e "$test_root/home/.local/bin/wayexpand-ibus" ]
[ ! -e "$test_root/home/.local/share/ibus/component/wayexpand.xml" ]
[ ! -e "$test_root/home/.local/share/applications/wayexpand.desktop" ]
[ ! -e "$test_root/home/.local/share/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml" ]
[ ! -e "$test_root/home/.local/share/man/man1/wayexpand.1" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-input-method.service" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-evdev.service" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-input-method.service.d/10-xdg-paths.conf" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-evdev.service.d/10-xdg-paths.conf" ]
[ ! -e "$test_root/config/systemd/user/wayexpand-action-broker.service.d/10-xdg-paths.conf" ]
[ -f "$test_root/config/wayexpand/expansions.toml" ]

# An enabled upgrade validates the existing library before replacing any
# installed files. A failed preflight must preserve the current daemon binary.
printf '%s\n' 'existing invalid configuration' >"$test_root/config/wayexpand/expansions.toml"
printf '%s\n' 'installed-daemon-sentinel' >"$test_root/home/.local/bin/wayexpand-daemon"
cat >"$release_dir/bin/wayexpand" <<'EOF'
#!/bin/sh
if [ "${1:-}" = validate ]; then
    exit 1
fi
exit 0
EOF
chmod 0755 "$release_dir/bin/wayexpand"
if PATH="$stub_bin:$PATH" \
    HOME="$test_root/home" \
    XDG_CONFIG_HOME="$test_root/config" \
    "$release_dir/scripts/install-release.sh" --enable --service=wayexpand-input-method.service \
    >/dev/null 2>&1; then
    printf '%s\n' 'installer accepted an invalid configuration during upgrade' >&2
    exit 1
fi
grep -Fx 'installed-daemon-sentinel' "$test_root/home/.local/bin/wayexpand-daemon" >/dev/null

# Restore the successful fake CLI for the remaining stop/uninstall tests.
cat >"$release_dir/bin/wayexpand" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 0755 "$release_dir/bin/wayexpand"

# A failed stop must fail closed before any executable is removed. This guards
# the rollback invariant that an active capture/injection process is never
# left running with its installed files silently deleted.
mkdir -p "$test_root/home/.local/bin"
printf '#!/bin/sh\nexit 0\n' >"$test_root/home/.local/bin/wayexpand-daemon"
chmod 0755 "$test_root/home/.local/bin/wayexpand-daemon"
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
case "$*" in
    "--user is-enabled "*) exit 0 ;;
    "--user disable --now "*) exit 1 ;;
    *) exit 1 ;;
esac
EOF
chmod 0755 "$stub_bin/systemctl"
if PATH="$stub_bin:$PATH" \
    HOME="$test_root/home" \
    XDG_CONFIG_HOME="$test_root/config" \
    "$release_dir/scripts/uninstall-user.sh" >/dev/null 2>&1; then
    printf '%s\n' 'uninstaller accepted a failed service stop' >&2
    exit 1
fi
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]

# A missing user systemd bus must not weaken the same invariant: a manually
# launched daemon is still an active input process and blocks uninstall.
# Simulate the missing bus with a stub. Moving the stub away would expose the
# real systemctl, which disabled the developer's own service in the past.
cat >"$stub_bin/systemctl" <<'EOF'
#!/bin/sh
printf '%s\n' 'Failed to connect to user scope bus via local transport: No such file or directory' >&2
exit 1
EOF
chmod 0755 "$stub_bin/systemctl"
cat >"$stub_bin/ps" <<'EOF'
#!/bin/sh
printf '%s\n' '4242 /tmp/wayexpand-daemon --config /tmp/test.toml'
printf '%s\n' '4243 /tmp/wayexpand-action-broker'
EOF
chmod 0755 "$stub_bin/ps"
if PATH="$stub_bin:$PATH" \
    HOME="$test_root/home" \
    XDG_CONFIG_HOME="$test_root/config" \
    "$release_dir/scripts/uninstall-user.sh" >/dev/null 2>&1; then
    printf '%s\n' 'uninstaller removed files while a manually launched daemon was active' >&2
    exit 1
fi
[ -x "$test_root/home/.local/bin/wayexpand-daemon" ]

printf '%s\n' "release install/uninstall test passed"
