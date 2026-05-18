#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
cc_bin=${CC:-cc}
pkg_config_bin=${PKG_CONFIG:-pkg-config}

skip() {
    echo "skip: $*" >&2
    exit 77
}

fail() {
    echo "error: $*" >&2
    if [ -f "${daemon_log:-}" ]; then
        echo "--- daemon.log ---" >&2
        cat "$daemon_log" >&2
    fi
    if [ -f "${provider_err:-}" ]; then
        echo "--- provider.err ---" >&2
        cat "$provider_err" >&2
    fi
    exit 1
}

command -v cargo >/dev/null || skip "cargo is required"
command -v "$cc_bin" >/dev/null || skip "cc is required"
command -v mountpoint >/dev/null || skip "mountpoint is required"
command -v fusermount3 >/dev/null || skip "fusermount3 is required"
command -v "$pkg_config_bin" >/dev/null || skip "pkg-config is required"
[ -e /dev/fuse ] || skip "/dev/fuse is required"
"$pkg_config_bin" --exists fuse3 || skip "fuse3 pkg-config metadata is required"

work_dir=$(mktemp -d)
runtime_dir="$work_dir/runtime"
mount_path="$runtime_dir/fuse-promise"
daemon_log="$work_dir/daemon.log"
provider_out="$work_dir/provider.out"
provider_err="$work_dir/provider.err"
provider_bin="$work_dir/minimal-provider"
provider_lib_dir="$work_dir/lib"
materialize_dir="$work_dir/materialized"
expected_file="$work_dir/expected.txt"
daemon_pid=
provider_pid=

cleanup() {
    set +e
    if [ -n "$provider_pid" ] && kill -0 "$provider_pid" 2>/dev/null; then
        kill "$provider_pid" 2>/dev/null
        wait "$provider_pid" 2>/dev/null
    fi
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill "$daemon_pid" 2>/dev/null
        wait "$daemon_pid" 2>/dev/null
    fi
    if [ -d "$mount_path" ] && mountpoint -q "$mount_path"; then
        fusermount3 -u "$mount_path" 2>/dev/null
    fi
    rm -rf "$work_dir"
}
trap cleanup EXIT

mkdir -m 700 "$runtime_dir"
mkdir "$provider_lib_dir" "$materialize_dir"
printf 'hello from fuse-promise example\n' > "$expected_file"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked
cargo build -p fuse-promise-daemon --features fuse-mount --locked

ln -s "$repo_dir/target/debug/libfusepromise.so" \
    "$provider_lib_dir/libfusepromise.so"
ln -s "$repo_dir/target/debug/libfusepromise.so" \
    "$provider_lib_dir/libfusepromise.so.0"
"$cc_bin" -std=c11 -Wall -Wextra -Werror -I"$repo_dir/include" \
    "$repo_dir/examples/minimal_provider.c" \
    -L"$provider_lib_dir" -lfusepromise \
    "-Wl,-rpath,$provider_lib_dir" \
    -o "$provider_bin"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fuse-promised" \
    --foreground > "$daemon_log" 2>&1 &
daemon_pid=$!

for _ in $(seq 1 100); do
    if [ -S "$runtime_dir/fuse-promise.sock" ] && mountpoint -q "$mount_path"; then
        break
    fi
    kill -0 "$daemon_pid" 2>/dev/null || fail "fuse-promised exited before mounting"
    sleep 0.1
done
mountpoint -q "$mount_path" || fail "mount did not become ready"

LD_LIBRARY_PATH="$provider_lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
    "$provider_bin" "$runtime_dir" > "$provider_out" 2> "$provider_err" &
provider_pid=$!

visible_path=
for _ in $(seq 1 100); do
    visible_path=$(sed -n '1p' "$provider_out")
    if [ -n "$visible_path" ]; then
        break
    fi
    kill -0 "$provider_pid" 2>/dev/null || fail "provider exited before commit"
    sleep 0.1
done
[ -n "$visible_path" ] || fail "provider did not print visible path"

promise_file="$visible_path/docs/hello.txt"
test -f "$promise_file" || fail "promised file is not visible"
stat -c '%F %s %a %Y' "$visible_path/docs" "$promise_file" > "$work_dir/stat.out"
grep -q '^directory 0 755 1700000000$' "$work_dir/stat.out" \
    || fail "promised directory attributes were not visible"
grep -q '^regular file 32 644 1700000000$' "$work_dir/stat.out" \
    || fail "promised file attributes were not visible"

cat "$promise_file" > "$work_dir/read.out"
cmp "$expected_file" "$work_dir/read.out" >/dev/null \
    || fail "promised file contents did not match provider data"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" \
    materialize "$promise_file" "$materialize_dir" > "$work_dir/materialize.out"
grep -q "^target_path=$materialize_dir/hello.txt$" "$work_dir/materialize.out" \
    || fail "materialize did not report expected target path"
cmp "$expected_file" "$materialize_dir/hello.txt" >/dev/null \
    || fail "materialized file contents did not match provider data"

echo "Minimal provider smoke passed"
