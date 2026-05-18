#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

prefix=${PREFIX:-/usr/local}
destdir=${DESTDIR:-}
bindir=${BINDIR:-"$prefix/bin"}
libdir=${LIBDIR:-"$prefix/lib"}
includedir=${INCLUDEDIR:-"$prefix/include"}
pkgconfigdir=${PKGCONFIGDIR:-"$libdir/pkgconfig"}
systemd_user_dir=${SYSTEMD_USER_DIR:-"$prefix/lib/systemd/user"}
daemon_features=${DAEMON_FEATURES:-}
soname_major=${SONAME_MAJOR:-0}

install_path() {
    printf '%s%s' "$destdir" "$1"
}

install_file() {
    local mode=$1
    local source=$2
    local target=$3
    install -D -m "$mode" "$source" "$(install_path "$target")"
}

install_text_template() {
    local mode=$1
    local source=$2
    local target=$3
    local rendered
    rendered=$(mktemp)
    sed \
        -e "s|@prefix@|$prefix|g" \
        -e "s|@version@|$version|g" \
        -e "s|@includedir@|$includedir|g" \
        -e "s|@libdir@|$libdir|g" \
        -e "s|@bindir@|$bindir|g" \
        "$source" > "$rendered"
    install_file "$mode" "$rendered" "$target"
    rm -f "$rendered"
}

cd "$repo_dir"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || {
    echo "install-dev: could not read workspace version" >&2
    exit 1
}

cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked
if [ -n "$daemon_features" ]; then
    cargo build -p fuse-promise-daemon --features "$daemon_features" --locked
else
    cargo build -p fuse-promise-daemon --locked
fi

install_file 0644 include/fuse-promise/fuse-promise.h \
    "$includedir/fuse-promise/fuse-promise.h"

versioned_lib="$libdir/libfusepromise.so.$version"
install_file 0755 target/debug/libfusepromise.so "$versioned_lib"
ln -sfn "libfusepromise.so.$version" "$(install_path "$libdir/libfusepromise.so.$soname_major")"
ln -sfn "libfusepromise.so.$soname_major" "$(install_path "$libdir/libfusepromise.so")"

install_text_template 0644 pkgconfig/fuse-promise.pc.in \
    "$pkgconfigdir/fuse-promise.pc"

install_file 0755 target/debug/fuse-promised "$bindir/fuse-promised"
install_file 0755 target/debug/fpctl "$bindir/fpctl"
install_text_template 0644 systemd/user/fuse-promised.service.in \
    "$systemd_user_dir/fuse-promised.service"

echo "installed fuse-promise $version into $(install_path "$prefix")"
