#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
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
    if [ -f "${read_log:-}" ]; then
        echo "--- read.log ---" >&2
        cat "$read_log" >&2
    fi
    exit 1
}

command -v cargo >/dev/null || skip "cargo is required"
command -v cc >/dev/null || skip "cc is required"
command -v mountpoint >/dev/null || skip "mountpoint is required"
command -v fusermount3 >/dev/null || skip "fusermount3 is required"
command -v "$pkg_config_bin" >/dev/null || skip "pkg-config is required"
[ -e /dev/fuse ] || skip "/dev/fuse is required"
"$pkg_config_bin" --exists fuse3 || skip "fuse3 pkg-config metadata is required"

work_dir=$(mktemp -d)
runtime_dir="$work_dir/runtime"
mount_path="$runtime_dir/fuse-promise"
read_log="$work_dir/read.log"
daemon_log="$work_dir/daemon.log"
provider_out="$work_dir/provider.out"
provider_err="$work_dir/provider.err"
provider_bin="$work_dir/read-only-mvp-provider"
provider_lib_dir="$work_dir/lib"
expected_file="$work_dir/expected.txt"
expected_stream="$work_dir/expected-stream.txt"
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
mkdir "$provider_lib_dir"
printf 'hello from fuse-promise\n' > "$expected_file"
printf 'abcdefghijklmnopqrstuvwxyz0123456789\n' > "$expected_stream"
: > "$read_log"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked
cargo build -p fuse-promise-daemon --features fuse-mount --locked

ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so"
ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so.0"
cc -I"$repo_dir/include" "$repo_dir/tests/read_only_mvp_provider.c" \
    -L"$provider_lib_dir" -lfusepromise \
    "-Wl,-rpath,$provider_lib_dir" \
    -o "$provider_bin"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fuse-promised" \
    --foreground --cache=read-through > "$daemon_log" 2>&1 &
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
    XDG_RUNTIME_DIR="$runtime_dir" "$provider_bin" "$read_log" \
    > "$provider_out" 2> "$provider_err" &
provider_pid=$!

visible_path=
for _ in $(seq 1 100); do
    visible_path=$(sed -n 's/^visible_path=//p' "$provider_out" | tail -n 1)
    if [ -n "$visible_path" ]; then
        break
    fi
    kill -0 "$provider_pid" 2>/dev/null || fail "provider exited before commit"
    sleep 0.1
done
[ -n "$visible_path" ] || fail "provider did not print visible path"

cached_path="$visible_path/stream.txt"
uncached_path="$visible_path/pending.txt"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" status \
    > "$work_dir/status.out"
grep -q '^mount=mounted$' "$work_dir/status.out" \
    || fail "fpctl status did not report mounted"
grep -q '^cache_policy=read-through$' "$work_dir/status.out" \
    || fail "fpctl status did not report read-through policy"

first_chunk=$(dd if="$cached_path" bs=1 count=4 status=none)
[ "$first_chunk" = "abcd" ] || fail "first sequential read returned unexpected data: $first_chunk"
read_count=$(wc -l < "$read_log")
[ "$read_count" -ge 2 ] || fail "first read did not trigger sequential prefetch"
grep -Eq '^READ offset=[1-9][0-9]* ' "$read_log" \
    || fail "sequential prefetch did not request the next range"

second_chunk=$(dd if="$cached_path" bs=1 skip=4 count=8 status=none)
[ "$second_chunk" = "efghijkl" ] \
    || fail "second sequential read returned unexpected data: $second_chunk"
second_read_count=$(wc -l < "$read_log")
[ "$second_read_count" = "$read_count" ] \
    || fail "prefetched cache hit still reached provider"

kill "$provider_pid"
wait "$provider_pid" || true
provider_pid=

for _ in $(seq 1 100); do
    if XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" list \
        | grep -q 'state=provider-gone'; then
        break
    fi
    sleep 0.1
done
XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" list \
    | grep -q 'state=provider-gone' || fail "provider disconnect did not mark promise provider-gone"

cat "$cached_path" > "$work_dir/after-disconnect-cached.out" \
    || fail "cached read after provider disconnect failed"
cmp "$expected_stream" "$work_dir/after-disconnect-cached.out" >/dev/null \
    || fail "cached read after provider disconnect returned unexpected data"

if cat "$uncached_path" > "$work_dir/after-disconnect-uncached.out" 2> "$work_dir/after-disconnect-uncached.err"; then
    fail "uncached read after provider disconnect unexpectedly succeeded"
fi

echo "Read-through cache smoke passed"
