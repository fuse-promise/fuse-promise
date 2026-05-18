#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
cc_bin=${CC:-cc}
pkg_config_bin=${PKG_CONFIG:-pkg-config}
readelf_bin=${READELF:-readelf}

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v "$cc_bin" >/dev/null || fail "cc is required"
command -v "$pkg_config_bin" >/dev/null || fail "pkg-config is required"
command -v "$readelf_bin" >/dev/null || fail "readelf is required"

work_dir=$(mktemp -d)
cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT

prefix="$work_dir/prefix"
build_dir="$work_dir/build"
runtime_dir="$work_dir/runtime"
mkdir -p "$build_dir" "$runtime_dir"
chmod 700 "$runtime_dir"

cd "$repo_dir"
PREFIX="$prefix" scripts/install-dev.sh > "$work_dir/install.log"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "could not read workspace version"

test -f "$prefix/include/fuse-promise/fuse-promise.h" \
    || fail "public header was not installed"
test -f "$prefix/lib/libfusepromise.so.$version" \
    || fail "versioned shared library was not installed"
"$readelf_bin" -d "$prefix/lib/libfusepromise.so.$version" \
    | grep -q 'SONAME.*libfusepromise.so.0' \
    || fail "shared library SONAME is not libfusepromise.so.0"
test -L "$prefix/lib/libfusepromise.so.0" \
    || fail "soname-major shared library link was not installed"
test -L "$prefix/lib/libfusepromise.so" \
    || fail "linker shared library link was not installed"
test -f "$prefix/lib/pkgconfig/fuse-promise.pc" \
    || fail "pkg-config file was not installed"
test -x "$prefix/bin/fuse-promised" \
    || fail "daemon binary was not installed"
test -x "$prefix/bin/fpctl" \
    || fail "fpctl binary was not installed"
test -f "$prefix/lib/systemd/user/fuse-promised.service" \
    || fail "systemd user service was not installed"

grep -q "^Version: $version$" "$prefix/lib/pkgconfig/fuse-promise.pc" \
    || fail "pkg-config version mismatch"
grep -q "^includedir=$prefix/include$" "$prefix/lib/pkgconfig/fuse-promise.pc" \
    || fail "pkg-config includedir mismatch"
grep -q "^libdir=$prefix/lib$" "$prefix/lib/pkgconfig/fuse-promise.pc" \
    || fail "pkg-config libdir mismatch"
grep -q "^ExecStart=$prefix/bin/fuse-promised --foreground$" \
    "$prefix/lib/systemd/user/fuse-promised.service" \
    || fail "systemd service ExecStart does not match installed bindir"
if grep -q '@' "$prefix/lib/systemd/user/fuse-promised.service"; then
    fail "systemd service contains unresolved template variables"
fi

export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
"$pkg_config_bin" --exists fuse-promise \
    || fail "installed pkg-config metadata is not discoverable"
pkg_flags=$("$pkg_config_bin" --cflags --libs fuse-promise)
case "$pkg_flags" in
    *"-I$prefix/include"*"-L$prefix/lib"*"-lfusepromise"*) ;;
    *) fail "pkg-config flags do not reference installed public ABI" ;;
esac

for example in examples/minimal_provider.c examples/materialize.c; do
    cp "$example" "$build_dir/"
    output="$build_dir/$(basename "$example" .c)"
    "$cc_bin" -std=c11 -Wall -Wextra -Werror \
        "$build_dir/$(basename "$example")" $pkg_flags \
        "-Wl,-rpath,$prefix/lib" \
        -o "$output"
    "$readelf_bin" -d "$output" | grep -q 'NEEDED.*libfusepromise.so.0' \
        || fail "$example does not depend on libfusepromise.so.0"
done

XDG_RUNTIME_DIR="$runtime_dir" "$prefix/bin/fpctl" status > "$work_dir/fpctl-status.out"
grep -q '^daemon=not-connected$' "$work_dir/fpctl-status.out" \
    || fail "installed fpctl status did not run"
grep -q '^cache_policy=no-cache$' "$work_dir/fpctl-status.out" \
    || fail "installed fpctl status did not report no-cache policy"
"$prefix/bin/fuse-promised" --help | grep -q '^usage: fuse-promised' \
    || fail "installed fuse-promised --help did not run"

stage="$work_dir/stage"
staged_prefix=/usr
DESTDIR="$stage" PREFIX="$staged_prefix" scripts/install-dev.sh \
    > "$work_dir/staged-install.log"
test -f "$stage$staged_prefix/include/fuse-promise/fuse-promise.h" \
    || fail "DESTDIR public header was not staged"
test -f "$stage$staged_prefix/lib/libfusepromise.so.$version" \
    || fail "DESTDIR shared library was not staged"
test -x "$stage$staged_prefix/bin/fuse-promised" \
    || fail "DESTDIR daemon binary was not staged"
test -x "$stage$staged_prefix/bin/fpctl" \
    || fail "DESTDIR fpctl binary was not staged"
grep -q "^includedir=$staged_prefix/include$" \
    "$stage$staged_prefix/lib/pkgconfig/fuse-promise.pc" \
    || fail "DESTDIR pkg-config includedir incorrectly includes staging root"
grep -q "^ExecStart=$staged_prefix/bin/fuse-promised --foreground$" \
    "$stage$staged_prefix/lib/systemd/user/fuse-promised.service" \
    || fail "DESTDIR systemd ExecStart incorrectly includes staging root"

echo "install metadata passed"
