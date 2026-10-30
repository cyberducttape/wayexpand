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
# Never reach the invoking user's real systemd manager or session bus, even
# where a test leaves systemctl unstubbed: any real `systemctl --user` then
# fails instead of stopping or disabling the developer's own services.
isolated_runtime="$test_root/runtime"
mkdir -m 0700 "$isolated_runtime"
export XDG_RUNTIME_DIR="$isolated_runtime"
export DBUS_SESSION_BUS_ADDRESS="unix:path=$isolated_runtime/bus"

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
    -p wayexpand -p wayexpand-daemon -p action-broker -p wayexpand-ui \
    -p wayexpand-gui -p wayexpand-backend-ibus -p wayexpand-e2e

build_triple=${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
case "$build_triple" in
    x86_64-*) release_arch=x86_64 ;;
    aarch64-*) release_arch=aarch64 ;;
    *) printf '%s\n' "unsupported release smoke-test target: $build_triple" >&2; exit 1 ;;
esac
package_output="$test_root/packages"
mkdir -m 0700 "$package_output"
(cd "$project_dir" && umask 077 && \
    CARGO_TARGET_DIR="$target_dir" \
    WAYEXPAND_RELEASE_OUTPUT_DIR="$package_output" \
    SOURCE_DATE_EPOCH=0 \
        scripts/package-release-binaries.sh test "$release_arch")
archive="$package_output/wayexpand-test-linux-$release_arch.tar.gz"
checksum="$archive.sha256"
repeat_output="$test_root/packages-repeat"
mkdir -m 0700 "$repeat_output"
(cd "$project_dir" && umask 002 && \
    CARGO_TARGET_DIR="$target_dir" \
    WAYEXPAND_RELEASE_OUTPUT_DIR="$repeat_output" \
    SOURCE_DATE_EPOCH=0 \
        scripts/package-release-binaries.sh test "$release_arch")
repeat_archive="$repeat_output/wayexpand-test-linux-$release_arch.tar.gz"
cmp "$archive" "$repeat_archive"
invalid_epoch_output="$test_root/packages-invalid-epoch"
mkdir -m 0700 "$invalid_epoch_output"
if (
    cd "$project_dir"
    CARGO_TARGET_DIR="$target_dir" \
    WAYEXPAND_RELEASE_OUTPUT_DIR="$invalid_epoch_output" \
    SOURCE_DATE_EPOCH=not-a-timestamp \
        scripts/package-release-binaries.sh test "$release_arch"
); then
    printf '%s\n' 'release packager accepted an invalid SOURCE_DATE_EPOCH' >&2
    exit 1
fi
[ ! -e "$invalid_epoch_output/wayexpand-test-linux-$release_arch" ]
(
    cd "$package_output"
    sha256sum -c "$(basename -- "$checksum")"
)
archive_listing="$test_root/release-archive.txt"
tar -tzf "$archive" >"$archive_listing"
extracted_archive="$test_root/extracted-release"
mkdir -m 0700 "$extracted_archive"
tar -xzf "$archive" -C "$extracted_archive"
for path in \
    "wayexpand-test-linux-$release_arch/bin/wayexpand" \
    "wayexpand-test-linux-$release_arch/bin/wayexpand-daemon" \
    "wayexpand-test-linux-$release_arch/bin/wayexpand-action-broker" \
    "wayexpand-test-linux-$release_arch/bin/wayexpand-ui" \
    "wayexpand-test-linux-$release_arch/bin/wayexpand-gui" \
    "wayexpand-test-linux-$release_arch/bin/wayexpand-ibus" \
    "wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/bin/wx-e2e-sink" \
    "wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/bin/wx-e2e-type" \
    "wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/bin/wx-e2e-eis-relay" \
    "wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/test-kwin-e2e.sh"; do
    grep -Fxq "$path" "$archive_listing" || {
        printf '%s\n' "release archive is missing $path" >&2
        exit 1
    }
    test -x "$extracted_archive/$path" || {
        printf '%s\n' "release archive entry is not executable: $path" >&2
        exit 1
    }
done
report_helper="wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/kwin-e2e-report.sh"
grep -Fxq "$report_helper" "$archive_listing" || {
    printf '%s\n' "release archive is missing $report_helper" >&2
    exit 1
}
test -f "$extracted_archive/$report_helper"
# Run the packaged harness far enough to resolve its adjacent sourced helper.
# An isolated PATH forces the predictable missing-compositor error without
# starting a desktop session or depending on the host's installed KWin tools.
probe_bin="$test_root/harness-probe-bin"
mkdir -m 0700 "$probe_bin"
ln -s "$(command -v bash)" "$probe_bin/bash"
ln -s "$(command -v dirname)" "$probe_bin/dirname"
if PATH="$probe_bin" "$extracted_archive/wayexpand-test-linux-$release_arch/libexec/wayexpand-certification/test-kwin-e2e.sh" \
    --source=stdin >"$test_root/harness-probe.out" 2>&1; then
    printf '%s\n' 'packaged KWin harness unexpectedly ran without compositor tools' >&2
    exit 1
fi
grep -F 'error: kwin_wayland is required' "$test_root/harness-probe.out" >/dev/null
test "$(stat -c '%a' "$extracted_archive/wayexpand-test-linux-$release_arch/expansions.toml")" = 600
packaged_cli="$extracted_archive/wayexpand-test-linux-$release_arch/bin/wayexpand"
"$packaged_cli" --version | grep -F 'wayexpand ' >/dev/null
"$packaged_cli" --help | grep -F 'doctor --certify' >/dev/null
if grep -E '(^|/)\.git(/|$)|(^|/)docs/archive(/|$)' "$archive_listing" >/dev/null; then
    printf '%s\n' 'release archive contains excluded repository content' >&2
    exit 1
fi

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
