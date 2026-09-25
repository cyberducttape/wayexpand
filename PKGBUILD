# Maintainer: Stephan Loesevitz <stephan.loesevitz at gmail dot com>
pkgname=wayexpand
pkgver=1.2.0
pkgrel=1
pkgdesc="A privacy-first, Wayland-native text expander for Linux"
arch=('x86_64')
url="https://github.com/cyberducttape/wayexpand"
license=('MIT')
depends=(
    'wayland'
    'libxkbcommon'
    'glibc'
)
makedepends=(
    'rust'
    'cargo'
    'pkg-config'
)
optdepends=(
    'systemd: for user service support'
    'ibus: for native GNOME IBus input method integration'
)
source=("https://github.com/cyberducttape/wayexpand/archive/v${pkgver}.tar.gz")
sha256sums=('9723b222eb717a15fb88b99526af99ad4d8a2c5ec538fa1b23bc17a6819b1056')
conflicts=('wayexpand-git')

prepare() {
    cd "${pkgname}-${pkgver}"
    cargo fetch --locked
}

build() {
    cd "${pkgname}-${pkgver}"
    cargo build --release --frozen -p wayexpand -p wayexpand-daemon -p wayexpand-ui -p wayexpand-gui -p wayexpand-backend-ibus
}

check() {
    cd "${pkgname}-${pkgver}"
    cargo test --release --frozen --workspace
}

package() {
    cd "${pkgname}-${pkgver}"

    # Install binaries
    install -Dm755 target/release/wayexpand "${pkgdir}/usr/bin/wayexpand"
    install -Dm755 target/release/wayexpand-daemon "${pkgdir}/usr/bin/wayexpand-daemon"
    install -Dm755 target/release/wayexpand-gui "${pkgdir}/usr/bin/wayexpand-gui"
    install -Dm755 target/release/wayexpand-ui "${pkgdir}/usr/bin/wayexpand-ui"
    install -Dm755 target/release/wayexpand-ibus "${pkgdir}/usr/bin/wayexpand-ibus"

    # Install desktop entry
    install -Dm644 desktop/wayexpand.desktop "${pkgdir}/usr/share/applications/wayexpand.desktop"
    install -Dm644 desktop/wayexpand-ibus.xml "${pkgdir}/usr/share/ibus/component/wayexpand-ibus.xml"
    install -Dm644 io.github.cyberducttape.WayExpand.metainfo.xml \
        "${pkgdir}/usr/share/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml"
    install -Dm644 docs/wayexpand.1 "${pkgdir}/usr/share/man/man1/wayexpand.1"

    # Install application icon at every size the desktop entry's Icon=
    # lookup can resolve to; without these the app shows a generic icon.
    for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
        install -Dm644 "assets/icon/hicolor/${size}/apps/wayexpand.png" \
            "${pkgdir}/usr/share/icons/hicolor/${size}/apps/wayexpand.png"
    done

    # Install systemd user units
    install -Dm644 systemd/wayexpand-input-method.service "${pkgdir}/usr/lib/systemd/user/wayexpand-input-method.service"
    install -Dm644 systemd/wayexpand-evdev.service "${pkgdir}/usr/lib/systemd/user/wayexpand-evdev.service"
    # The source units target the user-local install layout used by the
    # release scripts. Distro packages must bind them to the package-owned
    # binaries so ~/.local/bin cannot shadow an installed update.
    sed -i 's#%h/.local/bin/#/usr/bin/#g' \
        "${pkgdir}/usr/lib/systemd/user/wayexpand-input-method.service" \
        "${pkgdir}/usr/lib/systemd/user/wayexpand-evdev.service"

    # Ship evdev policies inertly. Installing WayExpand must not change raw
    # input authorization; the explicit helper copies a selected policy into
    # /etc/udev/rules.d when the user opts in.
    install -Dm644 udev/71-wayexpand-evdev.rules "${pkgdir}/usr/share/wayexpand/udev/71-wayexpand-evdev.rules"
    install -Dm644 udev/69-wayexpand-evdev-uaccess.rules "${pkgdir}/usr/share/wayexpand/udev/69-wayexpand-evdev-uaccess.rules"
    install -Dm755 scripts/install-evdev-permissions.sh "${pkgdir}/usr/bin/wayexpand-install-evdev-access"

    # Install documentation
    install -Dm644 README.md "${pkgdir}/usr/share/doc/wayexpand/README.md"
    install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/wayexpand/LICENSE"

    # Install example configuration
    install -Dm644 expansions.toml "${pkgdir}/etc/wayexpand/expansions.toml.example"
}
