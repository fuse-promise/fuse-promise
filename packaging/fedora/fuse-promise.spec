Name:           fuse-promise
Version:        1.0.3
Release:        1%{?dist}
Summary:        Promise filesystem runtime built on FUSE 3

License:        Apache-2.0
URL:            https://github.com/fuse-promise/fuse-promise
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz#/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(fuse3)
BuildRequires:  systemd-rpm-macros
Requires:       fuse3

%{!?_userunitdir:%global _userunitdir %{_prefix}/lib/systemd/user}

%description
fuse-promise provides a user-session FUSE daemon, a stable public C ABI
runtime library, and an administrative CLI for Promise filesystem providers.

%package devel
Summary:        Development files for fuse-promise
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       pkgconfig

%description devel
This package contains the public C header, unversioned shared library symlink,
and pkg-config metadata needed to build applications against libfusepromise.

%prep
%autosetup -n %{name}-%{version}

%build
FUSE_PROMISE_SONAME_MAJOR=1 cargo build -p fuse-promise-ffi --locked --release
cargo build -p fpctl --locked --release
cargo build -p fuse-promise-daemon --features fuse-mount-fuse3 --locked --release

%install
DESTDIR=%{buildroot} \
    PREFIX=%{_prefix} \
    LIBDIR=%{_libdir} \
    PKGCONFIGDIR=%{_libdir}/pkgconfig \
    SYSTEMD_USER_DIR=%{_userunitdir} \
    BUILD_PROFILE=release \
    SONAME_MAJOR=1 \
    DAEMON_FEATURES=fuse-mount-fuse3 \
    scripts/install-dev.sh

%check
cargo test --workspace --locked

%files
%license LICENSE
%doc README.md CHANGELOG.md
%{_bindir}/fpctl
%{_bindir}/fuse-promised
%{_libdir}/libfusepromise.so.1
%{_libdir}/libfusepromise.so.%{version}
%{_userunitdir}/fuse-promised.service

%files devel
%{_includedir}/fuse-promise/fuse-promise.h
%{_libdir}/libfusepromise.so
%{_libdir}/pkgconfig/fuse-promise.pc

%changelog
* Tue May 19 2026 fuse-promise contributors <maintainers@fuse-promise.invalid> - 1.0.3-1
- Initial Fedora package.
