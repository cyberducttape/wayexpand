Name:           wayexpand
Version:        0.2.0
Release:        1%{?dist}
Summary:        A privacy-first, Wayland-native text expander for Linux
License:        MIT
URL:            https://github.com/itchyitchy123/wayexpand

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
export CARGO_NET_OFFLINE=true
cargo build --release --locked --all

%check
cargo test --release --locked --workspace

%install
install -Dm755 target/release/wayexpand %{buildroot}%{_bindir}/wayexpand
install -Dm755 target/release/wayexpand-daemon %{buildroot}%{_bindir}/wayexpand-daemon
install -Dm755 target/release/wayexpand-gui %{buildroot}%{_bindir}/wayexpand-gui
install -Dm755 target/release/wayexpand-ui %{buildroot}%{_bindir}/wayexpand-ui

install -Dm644 desktop/wayexpand.desktop %{buildroot}%{_datadir}/applications/wayexpand.desktop

install -Dm644 systemd/wayexpand.service %{buildroot}%{_userunitdir}/wayexpand.service
install -Dm644 systemd/wayexpand-input-method.service %{buildroot}%{_userunitdir}/wayexpand-input-method.service
install -Dm644 systemd/wayexpand-evdev.service %{buildroot}%{_userunitdir}/wayexpand-evdev.service

install -Dm644 udev/71-wayexpand-evdev.rules %{buildroot}%{_udevrulesdir}/71-wayexpand-evdev.rules

install -Dm644 expansions.toml %{buildroot}%{_sysconfdir}/wayexpand/expansions.toml.example
install -Dm644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license LICENSE
%doc README.md
%{_bindir}/wayexpand
%{_bindir}/wayexpand-daemon
%{_bindir}/wayexpand-gui
%{_bindir}/wayexpand-ui
%{_datadir}/applications/wayexpand.desktop
%{_userunitdir}/wayexpand.service
%{_userunitdir}/wayexpand-input-method.service
%{_userunitdir}/wayexpand-evdev.service
%{_udevrulesdir}/71-wayexpand-evdev.rules
%config(noreplace) %{_sysconfdir}/wayexpand/expansions.toml.example

%changelog
* Mon Sep 16 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 0.2.0-1
- Release v0.2.0: evdev backend, GUI redesign, character-drop fix

* Tue Sep 10 2026 Stephan Loesevitz <stephan.loesevitz@gmail.com> - 0.1.0-1
- Initial Fedora package release
