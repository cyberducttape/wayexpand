#!/bin/bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
    printf '%s\n' "usage: $0 <version> <architecture>" >&2
    exit 2
fi

version=$1
architecture=$2
project_dir=$(pwd -P)
output_dir=${WAYEXPAND_RELEASE_OUTPUT_DIR:-$project_dir}
mkdir -p "$output_dir"
case "$architecture" in
    x86_64|aarch64) ;;
    *) printf '%s\n' "unsupported release architecture: $architecture" >&2; exit 2 ;;
esac
host_triple=$(rustc -vV | sed -n 's/^host: //p')
case "$host_triple" in
    "${architecture}-"*) ;;
    *)
        printf '%s\n' "requested archive architecture $architecture does not match Rust host $host_triple" >&2
        exit 1
        ;;
esac

root_name="wayexpand-${version}-linux-${architecture}"
root="$output_dir/$root_name"
archive="$root.tar.gz"
if [ -e "$root" ] || [ -e "$archive" ]; then
    printf '%s\n' "release output already exists: $root" >&2
    exit 1
fi

mkdir -p "$root/bin" "$root/systemd" "$root/desktop" "$root/docs" \
    "$root/scripts" "$root/udev" "$root/ibus/component"
for binary in wayexpand wayexpand-daemon wayexpand-action-broker wayexpand-ui wayexpand-gui wayexpand-ibus; do
install -m 0755 "$project_dir/target/release/$binary" "$root/bin/"
done
install -m 0644 "$project_dir/systemd/wayexpand-input-method.service" \
    "$project_dir/systemd/wayexpand-evdev.service" \
    "$project_dir/systemd/wayexpand-action-broker.service" "$root/systemd/"
install -m 0644 "$project_dir/desktop/wayexpand.desktop" "$root/desktop/"
install -m 0644 "$project_dir/desktop/wayexpand-ibus.xml" "$root/ibus/component/"
install -m 0644 "$project_dir/io.github.cyberducttape.WayExpand.metainfo.xml" "$root/"
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    mkdir -p "$root/icons/hicolor/$size/apps"
    install -m 0644 "$project_dir/assets/icon/hicolor/$size/apps/wayexpand.png" \
        "$root/icons/hicolor/$size/apps/wayexpand.png"
done
install -m 0644 "$project_dir/README.md" "$project_dir/LICENSE" \
    "$project_dir/SECURITY.md" "$project_dir/CHANGELOG.md" "$root/"
install -m 0600 "$project_dir/expansions.toml" "$root/expansions.toml"
install -m 0600 "$project_dir/broker.toml.example" "$root/broker.toml.example"
install -m 0755 "$project_dir/scripts/install-release.sh" "$root/scripts/"
install -m 0755 "$project_dir/scripts/uninstall-user.sh" "$root/scripts/"
install -m 0755 "$project_dir/scripts/install-evdev-permissions.sh" "$root/scripts/"
install -m 0644 "$project_dir/udev/71-wayexpand-evdev.rules" "$root/udev/"
install -m 0644 "$project_dir/udev/69-wayexpand-evdev-uaccess.rules" "$root/udev/"
cp -R "$project_dir/docs/." "$root/docs/"
rm -rf "$root/docs/archive"
test ! -e "$root/docs/archive"

tar -czf "$archive" -C "$output_dir" "$root_name"
if tar -tzf "$archive" | grep -E '(^|/)\.git(/|$)|(^|/)docs/archive(/|$)' >/dev/null; then
    printf '%s\n' "release archive contains excluded repository content" >&2
    exit 1
fi
(cd "$output_dir" && sha256sum "$root_name.tar.gz" > "$root_name.tar.gz.sha256")
