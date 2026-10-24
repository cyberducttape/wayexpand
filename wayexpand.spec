Name:           wayexpand
Version:        1.3.3
Release:        1%{?dist}
Summary:        A privacy-first, Wayland-native text expander for Linux
License:        MIT
URL:            https://github.com/cyberducttape/wayexpand
Source0:        %{url}/releases/download/v%{version}/wayexpand-%{version}-vendored.tar.gz

%{!?_userunitdir:%global _userunitdir %{_prefix}/lib/systemd/user}

BuildRequires:  cargo
BuildRequires:  rustc
BuildRequires:  pkg-config
BuildRequires:  wayland-devel
BuildRequires:  libxkbcommon-devel
Requires:       wayland-libs
Requires:       libxkbcommon

%description
WayExpand is a local Wayland text expander with a graphical editor, CLI,
snippet templates, and optional command-backed expansions.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --frozen -p wayexpand -p wayexpand-daemon -p action-broker -p wayexpand-ui -p wayexpand-gui -p wayexpand-backend-ibus -p wayexpand-e2e

%check
cargo test --frozen --workspace

%install
install -Dm755 target/release/wayexpand %{buildroot}%{_bindir}/wayexpand
install -Dm755 target/release/wayexpand-daemon %{buildroot}%{_bindir}/wayexpand-daemon
install -Dm755 target/release/wayexpand-action-broker %{buildroot}%{_bindir}/wayexpand-action-broker
install -Dm755 target/release/wayexpand-gui %{buildroot}%{_bindir}/wayexpand-gui
install -Dm755 target/release/wayexpand-ui %{buildroot}%{_bindir}/wayexpand-ui
install -Dm755 target/release/wayexpand-ibus %{buildroot}%{_bindir}/wayexpand-ibus
install -Dm755 target/release/wx-e2e-sink %{buildroot}%{_libexecdir}/wayexpand-certification/bin/wx-e2e-sink
install -Dm755 target/release/wx-e2e-type %{buildroot}%{_libexecdir}/wayexpand-certification/bin/wx-e2e-type
install -Dm755 target/release/wx-e2e-eis-relay %{buildroot}%{_libexecdir}/wayexpand-certification/bin/wx-e2e-eis-relay
install -Dm755 scripts/test-kwin-e2e.sh %{buildroot}%{_libexecdir}/wayexpand-certification/test-kwin-e2e.sh
install -Dm755 scripts/kwin-e2e-session.sh %{buildroot}%{_libexecdir}/wayexpand-certification/kwin-e2e-session.sh
install -Dm644 desktop/wayexpand.desktop %{buildroot}%{_datadir}/applications/wayexpand.desktop
install -Dm644 desktop/wayexpand-ibus.xml %{buildroot}%{_datadir}/ibus/component/wayexpand-ibus.xml
install -Dm644 io.github.cyberducttape.WayExpand.metainfo.xml %{buildroot}%{_datadir}/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml
install -Dm644 docs/wayexpand.1 %{buildroot}%{_mandir}/man1/wayexpand.1
for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    install -Dm644 assets/icon/hicolor/${size}/apps/wayexpand.png %{buildroot}%{_datadir}/icons/hicolor/${size}/apps/wayexpand.png
done
install -Dm644 systemd/wayexpand-input-method.service %{buildroot}%{_userunitdir}/wayexpand-input-method.service
install -Dm644 systemd/wayexpand-evdev.service %{buildroot}%{_userunitdir}/wayexpand-evdev.service
install -Dm644 systemd/wayexpand-action-broker.service %{buildroot}%{_userunitdir}/wayexpand-action-broker.service
sed -i -e 's#%h/.local/lib/wayexpand/current/bin/#/usr/bin/#g' -e 's#%h/.local/bin/#/usr/bin/#g' \
    %{buildroot}%{_userunitdir}/wayexpand-input-method.service \
    %{buildroot}%{_userunitdir}/wayexpand-evdev.service \
    %{buildroot}%{_userunitdir}/wayexpand-action-broker.service
install -Dm644 udev/71-wayexpand-evdev.rules %{buildroot}%{_datadir}/wayexpand/udev/71-wayexpand-evdev.rules
install -Dm644 udev/69-wayexpand-evdev-uaccess.rules %{buildroot}%{_datadir}/wayexpand/udev/69-wayexpand-evdev-uaccess.rules
install -Dm755 scripts/install-evdev-permissions.sh %{buildroot}%{_bindir}/wayexpand-install-evdev-access
install -Dm644 expansions.toml %{buildroot}%{_sysconfdir}/wayexpand/expansions.toml.example
install -Dm600 broker.toml.example %{buildroot}%{_sysconfdir}/wayexpand/broker.toml.example
install -Dm644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license %{_licensedir}/%{name}/LICENSE
%doc README.md
%{_bindir}/wayexpand
%{_bindir}/wayexpand-daemon
%{_bindir}/wayexpand-action-broker
%{_bindir}/wayexpand-gui
%{_bindir}/wayexpand-ui
%{_bindir}/wayexpand-ibus
%{_bindir}/wayexpand-install-evdev-access
%{_libexecdir}/wayexpand-certification/bin/wx-e2e-sink
%{_libexecdir}/wayexpand-certification/bin/wx-e2e-type
%{_libexecdir}/wayexpand-certification/bin/wx-e2e-eis-relay
%{_libexecdir}/wayexpand-certification/test-kwin-e2e.sh
%{_libexecdir}/wayexpand-certification/kwin-e2e-session.sh
%{_datadir}/ibus/component/wayexpand-ibus.xml
%{_datadir}/applications/wayexpand.desktop
%{_datadir}/metainfo/io.github.cyberducttape.WayExpand.metainfo.xml
%{_mandir}/man1/wayexpand.1*
%{_userunitdir}/wayexpand-input-method.service
%{_userunitdir}/wayexpand-evdev.service
%{_userunitdir}/wayexpand-action-broker.service
%{_datadir}/wayexpand/udev/71-wayexpand-evdev.rules
%{_datadir}/wayexpand/udev/69-wayexpand-evdev-uaccess.rules
%config(noreplace) %{_sysconfdir}/wayexpand/expansions.toml.example
%config(noreplace) %{_sysconfdir}/wayexpand/broker.toml.example
%{_datadir}/icons/hicolor/*/apps/wayexpand.png

%changelog
* Sat Oct 03 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.3.3-1
- Release v1.3.3

* Sat Oct 03 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.3.2-1
- Release v1.3.2

* Sat Oct 03 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.3.1-1
- Release v1.3.1

* Sat Oct 03 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.3.0-1
- Release v1.3.0

* Thu Sep 24 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 1.2.0-1
- Release v1.2.0: backend correctness, policy enforcement, and release hardening
