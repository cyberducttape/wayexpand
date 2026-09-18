# Maintainer: Stephan Loesevitz <stephan.loesevitz at gmail dot com>
pkgname=wayexpand
pkgver=1.1.1
pkgrel=1
pkgdesc="A privacy-first, Wayland-native text expander for Linux"
arch=('x86_64')
url="https://github.com/itchyitchy123/wayexpand"
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
)
source=("https://github.com/itchyitchy123/wayexpand/archive/v${pkgver}.tar.gz")
# SKIP is deliberate for a locally-built (makepkg -si) package: pacman
# still verifies the download against it once a real value is filled in.
# Before submitting/updating this PKGBUILD on the AUR, replace SKIP with
# `updpkgsums` output (or `makepkg -g`) so AUR installs get real
# tamper-detection, not just for local builds.
sha256sums=('SKIP')
conflicts=('wayexpand-git')

build() {
    cd "${pkgname}-${pkgver}"
    cargo build --release --locked --all
}

check() {
    cd "${pkgname}-${pkgver}"
    cargo test --release --locked --workspace
}

package() {
    cd "${pkgname}-${pkgver}"

    # Install binaries
    install -Dm755 target/release/wayexpand "${pkgdir}/usr/bin/wayexpand"
    install -Dm755 target/release/wayexpand-daemon "${pkgdir}/usr/bin/wayexpand-daemon"
    install -Dm755 target/release/wayexpand-gui "${pkgdir}/usr/bin/wayexpand-gui"
    install -Dm755 target/release/wayexpand-ui "${pkgdir}/usr/bin/wayexpand-ui"

    # Install desktop entry
    install -Dm644 desktop/wayexpand.desktop "${pkgdir}/usr/share/applications/wayexpand.desktop"

    # Install application icon at every size the desktop entry's Icon=
    # lookup can resolve to; without these the app shows a generic icon.
    for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
        install -Dm644 "assets/icon/hicolor/${size}/apps/wayexpand.png" \
            "${pkgdir}/usr/share/icons/hicolor/${size}/apps/wayexpand.png"
    done

    # Install systemd user units
    install -Dm644 systemd/wayexpand.service "${pkgdir}/usr/lib/systemd/user/wayexpand.service"
    install -Dm644 systemd/wayexpand-input-method.service "${pkgdir}/usr/lib/systemd/user/wayexpand-input-method.service"
    install -Dm644 systemd/wayexpand-evdev.service "${pkgdir}/usr/lib/systemd/user/wayexpand-evdev.service"

    # Install udev rules for evdev backend
    install -Dm644 udev/71-wayexpand-evdev.rules "${pkgdir}/usr/lib/udev/rules.d/71-wayexpand-evdev.rules"

    # Install documentation
    install -Dm644 README.md "${pkgdir}/usr/share/doc/wayexpand/README.md"
    install -Dm644 LICENSE "${pkgdir}/usr/share/licenses/wayexpand/LICENSE"

    # Install example configuration
    install -Dm644 expansions.toml "${pkgdir}/etc/wayexpand/expansions.toml.example"
}
