#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
cc_bin=${CC:-cc}
pkg_config_bin=${PKG_CONFIG:-pkg-config}
nm_bin=${NM:-nm}

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v cargo >/dev/null || fail "cargo is required"
command -v "$cc_bin" >/dev/null || fail "cc is required"
command -v "$pkg_config_bin" >/dev/null || fail "pkg-config is required"
command -v "$nm_bin" >/dev/null || fail "nm is required"

work_dir=$(mktemp -d)
cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT

prefix="$work_dir/prefix"
mkdir -p "$prefix/include/fuse-promise" "$prefix/lib/pkgconfig"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked

cp include/fuse-promise/fuse-promise.h "$prefix/include/fuse-promise/fuse-promise.h"
cp target/debug/libfusepromise.so "$prefix/lib/libfusepromise.so"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "could not read workspace version"
sed -e "s|@prefix@|$prefix|g" -e "s|@version@|$version|g" \
    pkgconfig/fuse-promise.pc.in > "$prefix/lib/pkgconfig/fuse-promise.pc"

export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
"$pkg_config_bin" --cflags --libs fuse-promise > "$work_dir/pkg-config.flags"
grep -q -- "-I$prefix/include" "$work_dir/pkg-config.flags" \
    || fail "pkg-config output missing include path"
grep -q -- "-L$prefix/lib" "$work_dir/pkg-config.flags" \
    || fail "pkg-config output missing library path"
grep -q -- "-lfusepromise" "$work_dir/pkg-config.flags" \
    || fail "pkg-config output missing library"

"$nm_bin" -D --defined-only target/debug/libfusepromise.so \
    | awk '{print $3}' | sort > "$work_dir/symbols.actual"
cat > "$work_dir/symbols.expected" <<'SYMBOLS'
fp_context_close
fp_context_open
fp_materialize
fp_promise_add_dir
fp_promise_add_file
fp_promise_builder_free
fp_promise_builder_new
fp_promise_commit
fp_provider_register
fp_provider_unregister
fp_status_string
SYMBOLS
if ! cmp -s "$work_dir/symbols.expected" "$work_dir/symbols.actual"; then
    diff -u "$work_dir/symbols.expected" "$work_dir/symbols.actual" >&2 || true
    fail "public symbol exports differ from expected fp_ ABI"
fi

pkg_flags=$("$pkg_config_bin" --cflags --libs fuse-promise)
"$cc_bin" -std=c11 -Wall -Wextra -Werror \
    tests/abi_public_surface.c $pkg_flags \
    "-Wl,-rpath,$prefix/lib" \
    -o "$work_dir/abi_public_surface"
LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$work_dir/abi_public_surface"

for example in examples/minimal_provider.c examples/materialize.c; do
    if grep -Eq '#include[[:space:]]+"' "$example"; then
        fail "$example includes a quoted project-local header"
    fi
    project_includes=$(grep -E '#include[[:space:]]+<fuse-promise/' "$example" || true)
    if [ "$project_includes" != '#include <fuse-promise/fuse-promise.h>' ]; then
        fail "$example must include only fuse-promise/fuse-promise.h from this project"
    fi
    "$cc_bin" -std=c11 -Wall -Wextra -Werror "$example" $pkg_flags \
        "-Wl,-rpath,$prefix/lib" \
        -o "$work_dir/$(basename "$example" .c)"
done

echo "ABI hardening passed"
