#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

skip() {
    echo "skip: $*" >&2
    exit 77
}

fail() {
    echo "error: $*" >&2
    if [ -f "${ffi_out:-}" ]; then
        echo "--- ffi.out ---" >&2
        cat "$ffi_out" >&2
    fi
    if [ -f "${ffi_err:-}" ]; then
        echo "--- ffi.err ---" >&2
        cat "$ffi_err" >&2
    fi
    if [ -f "${fpctl_err:-}" ]; then
        echo "--- fpctl.err ---" >&2
        cat "$fpctl_err" >&2
    fi
    exit 1
}

command -v cargo >/dev/null || skip "cargo is required"
command -v cc >/dev/null || skip "cc is required"

work_dir=$(mktemp -d)
runtime_dir="$work_dir/runtime"
target_dir="$work_dir/target"
target_link="$work_dir/target-link"
ffi_bin="$work_dir/materialize-security"
ffi_out="$work_dir/ffi.out"
ffi_err="$work_dir/ffi.err"
fpctl_err="$work_dir/fpctl.err"
provider_lib_dir="$work_dir/lib"
promise_path="/tmp/fuse-promise/promise-1/readme.txt"

cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT

mkdir -m 700 "$runtime_dir"
mkdir "$target_dir" "$provider_lib_dir"
ln -s "$target_dir" "$target_link"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked

ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so"
ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so.0"
cc -std=c11 -Wall -Wextra -Werror -I"$repo_dir/include" \
    "$repo_dir/tests/materialize_security.c" \
    -L"$provider_lib_dir" -lfusepromise \
    "-Wl,-rpath,$provider_lib_dir" \
    -o "$ffi_bin"

LD_LIBRARY_PATH="$provider_lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    XDG_RUNTIME_DIR="$runtime_dir" "$ffi_bin" "$promise_path" "$target_link" \
    > "$ffi_out" 2> "$ffi_err" \
    || fail "C ABI materialize did not reject a symlink target directory"
grep -q '^status=invalid argument$' "$ffi_out" \
    || fail "C ABI materialize returned an unexpected status"

if XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" \
    materialize "$promise_path" "$target_link" > "$work_dir/fpctl.out" \
    2> "$fpctl_err"; then
    fail "fpctl materialize accepted a symlink target directory"
fi
grep -q "target directory must not be a symlink" "$fpctl_err" \
    || fail "fpctl materialize did not report the symlink target directory"

echo "Materialize security passed"
