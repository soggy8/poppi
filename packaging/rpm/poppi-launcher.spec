# Spec for Fedora (local builds and COPR). Not yet verified with rpmbuild;
# it downloads crates during the build, so it needs network access
# (in COPR: enable "Enable internet access during builds").
#
#   git archive --prefix=poppi-launcher-0.3.0/ -o ~/rpmbuild/SOURCES/poppi-launcher-0.3.0.tar.gz HEAD
#   rpmbuild -bb packaging/rpm/poppi-launcher.spec

Name:           poppi-launcher
Version:        0.3.0
Release:        1%{?dist}
Summary:        Fast application launcher for GNOME

License:        MIT
URL:            https://github.com/soggy8/poppi
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gtk4-devel
Requires:       gtk4
Recommends:     wl-clipboard
Recommends:     curl
Recommends:     libsecret
Suggests:       wmctrl
Suggests:       xdotool

%description
Poppi Launcher searches applications, files and open windows, calculates and
converts units and currencies, picks emojis, runs terminal commands and
answers questions through local or cloud AI models, from one search bar.

%prep
%autosetup

%build
cargo build --release --locked

%check
cargo test --release --locked

%install
install -Dm755 target/release/poppi_launcher %{buildroot}%{_bindir}/poppi_launcher
install -Dm644 data/com.poppi.launcher.desktop %{buildroot}%{_datadir}/applications/com.poppi.launcher.desktop
install -Dm644 data/com.poppi.launcher.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.poppi.launcher.svg

%files
%license LICENSE
%doc README.md
%{_bindir}/poppi_launcher
%{_datadir}/applications/com.poppi.launcher.desktop
%{_datadir}/icons/hicolor/scalable/apps/com.poppi.launcher.svg

%changelog
* Thu Nov 12 2026 andrej - 0.3.0-1
- Multi-provider AI models, keyring support, code block copy and faster indexing

* Wed Nov 11 2026 andrej - 0.2.0-1
- Initial package
