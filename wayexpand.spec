Name:           wayexpand
Version:        1.2.0
Release:        1%{?dist}
Summary:        A privacy-first, Wayland-native text expander for Linux
License:        MIT
URL:            https://github.com/cyberducttape/wayexpand

Source0:        %{url}/archive/v%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rustc
BuildRequires:  pkg-config
BuildRequires:  wayland-devel
BuildRequires:  libxkbcommon-devel

Requires:       wayland-libs
Requires:       libxkbcommon

%description
WayExpand is a privacy-first text expander designed for Wayland desktop
environments. Type a short trigger like ;;hello and it automatically expands
to your saved snippet.

Features:
  - Wayland-native with support for multiple compositors
  - Native graphical editor (GUI) and command-line interface (CLI)
  - No cloud, no telemetry, configuration stored locally
  - Experimental support for compositors without input-method-v2
  - Espanso configuration import support

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --frozen -p wayexpand -p wayexpand-daemon -p wayexpand-ui -p wayexpand-gui -p wayexpand-backend-ibus

%check
cargo test --release --frozen --workspace

%install
install -Dm755 target/release/wayexpand %{buildroot}%{_bindir}/wayexpand
install -Dm755 target/release/wayexpand-daemon %{buildroot}%{_bindir}/wayexpand-daemon
install -Dm755 target/release/wayexpand-gui %{buildroot}%{_bindir}/wayexpand-gui
install -Dm755 target/release/wayexpand-ui %{buildroot}%{_bindir}/wayexpand-ui
install -Dm755 target/release/wayexpand-ibus %{buildroot}%{_bindir}/wayexpand-ibus

install -Dm644 desktop/wayexpand.desktop %{buildroot}%{_datadir}/applications/wayexpand.desktop
install -Dm644 desktop/wayexpand-ibus.xml %{buildroot}%{_datadir}/ibus/component/wayexpand-ibus.xml
install -Dm644 io.github.cyberducttape.WayExpand.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml
install -Dm644 docs/wayexpand.1 %{buildroot}%{_mandir}/man1/wayexpand.1

for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    install -Dm644 assets/icon/hicolor/${size}/apps/wayexpand.png \
        %{buildroot}%{_datadir}/icons/hicolor/${size}/apps/wayexpand.png
done

install -Dm644 systemd/wayexpand-input-method.service %{buildroot}%{_userunitdir}/wayexpand-input-method.service
install -Dm644 systemd/wayexpand-evdev.service %{buildroot}%{_userunitdir}/wayexpand-evdev.service
sed -i 's#%h/.local/bin/#/usr/bin/#g' \
    %{buildroot}%{_userunitdir}/wayexpand-input-method.service \
    %{buildroot}%{_userunitdir}/wayexpand-evdev.service

# Ship evdev policies inertly. Installing WayExpand must not change raw input
# authorization; the explicit helper copies a selected policy into
# /etc/udev/rules.d when the user opts in.
install -Dm644 udev/71-wayexpand-evdev.rules %{buildroot}%{_datadir}/wayexpand/udev/71-wayexpand-evdev.rules
install -Dm644 udev/69-wayexpand-evdev-uaccess.rules %{buildroot}%{_datadir}/wayexpand/udev/69-wayexpand-evdev-uaccess.rules
install -Dm755 scripts/install-evdev-permissions.sh %{buildroot}%{_bindir}/wayexpand-install-evdev-access

install -Dm644 expansions.toml %{buildroot}%{_sysconfdir}/wayexpand/expansions.toml.example
install -Dm644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license LICENSE
%doc README.md
%{_bindir}/wayexpand
%{_bindir}/wayexpand-daemon
%{_bindir}/wayexpand-gui
%{_bindir}/wayexpand-ui
%{_bindir}/wayexpand-ibus
%{_datadir}/ibus/component/wayexpand-ibus.xml
%{_datadir}/applications/wayexpand.desktop
%{_datadir}/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml
%{_mandir}/man1/wayexpand.1
%{_userunitdir}/wayexpand-input-method.service
%{_userunitdir}/wayexpand-evdev.service
%{_datadir}/wayexpand/udev/71-wayexpand-evdev.rules
%{_datadir}/wayexpand/udev/69-wayexpand-evdev-uaccess.rules
%config(noreplace) %{_sysconfdir}/wayexpand/expansions.toml.example
%{_datadir}/icons/hicolor/*/apps/wayexpand.png

%changelog
* Thu Sep 24 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.2.0-1
- Release v1.2.0: backend correctness, policy enforcement, and release hardening
- Add bounded asynchronous command processing and input pass-through safeguards
- Improve compositor capability reporting, fleet policy handling, and packaging

* Thu Sep 19 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.1.2-1
- Release v1.1.2: security hardening and correctness fixes
- Fix P0: Cross-window buffer isolation, pause/sensitive-field state,
  ancestor path validation, clipboard fallback, and input-method-v2 key loss
- Add 170+ regression tests for security fixes

* Thu Sep 18 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.1.1-1
- Release v1.1.1: font scaling, retro color themes, WCAG AA contrast fixes,
  sysadmin snippet examples

* Mon Sep 16 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 0.2.0-1
- Release v0.2.0: evdev backend, GUI redesign, character-drop fix

* Tue Sep 10 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 0.1.0-1
- Initial Fedora package release
