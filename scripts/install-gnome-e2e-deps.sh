#!/bin/sh
# Install what scripts/test-gnome-e2e.sh needs inside a CI container: GNOME
# Shell, IBus, the GTK, Qt, terminal and browser test clients, and the build
# toolchain prerequisites. Used by the per-Shell-version GNOME jobs.
set -eu

# shellcheck source=/dev/null
. /etc/os-release

case "$ID" in
    fedora)
        dnf install -y chromium curl dbus-daemon dconf gcc gcc-c++ git glib2 gtk3 gtk4 \
            gnome-shell gsettings-desktop-schemas gzip ibus ibus-gtk3 ibus-gtk4 \
            libei-devel libinput-devel libudev-devel libxkbcommon-devel \
            mesa-dri-drivers pkgconf-pkg-config python3 python3-gobject \
            python3-pyqt6 python3-websocket-client shadow-utils systemd tar \
            wayland-devel xdotool xprop xdpyinfo xorg-x11-server-Xwayland xterm
        ;;
    debian | ubuntu)
        export DEBIAN_FRONTEND=noninteractive
        apt-get update
        apt-get install -y --no-install-recommends ca-certificates chromium curl \
            dbus-daemon dconf-service g++ gcc git gir1.2-gtk-3.0 gir1.2-gtk-4.0 \
            gnome-shell gsettings-desktop-schemas ibus ibus-gtk3 ibus-gtk4 \
            libc6-dev libei-dev libgl1-mesa-dri libglib2.0-bin libinput-dev libudev-dev \
            libwayland-dev libxkbcommon-dev locales passwd pkg-config procps python3 \
            python3-gi python3-pyqt6 python3-websocket systemd util-linux x11-utils \
            xdotool xterm xwayland
        ;;
    *)
        printf '%s\n' "error: no GNOME test dependency list for $ID" >&2
        exit 2
        ;;
esac

id wayexpand-test >/dev/null 2>&1 || useradd --create-home wayexpand-test
