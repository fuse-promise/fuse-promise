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
expected_file="$work_dir/expected.txt"
copy_file="$work_dir/copied.txt"
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
printf 'hello from fuse-promise\n' > "$expected_file"
: > "$read_log"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked
cargo build -p fuse-promise-daemon --features fuse-mount --locked

cc -I"$repo_dir/include" "$repo_dir/tests/read_only_mvp_provider.c" \
    -L"$repo_dir/target/debug" -lfusepromise \
    "-Wl,-rpath,$repo_dir/target/debug" \
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

LD_LIBRARY_PATH="$repo_dir/target/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
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
[ "$visible_path" = "$mount_path/promise-1" ] || fail "unexpected visible path: $visible_path"

file_path="$visible_path/docs/readme.txt"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" status \
    | grep -q '^mount=mounted$' || fail "fpctl status did not report mounted"
XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" list \
    | grep -q '^promises=1$' || fail "fpctl list did not report one promise"

find "$visible_path" -maxdepth 3 -printf '%y %P\n' | sort > "$work_dir/find.out"
grep -q '^d docs$' "$work_dir/find.out" || fail "find did not see docs directory"
grep -q '^f docs/readme.txt$' "$work_dir/find.out" || fail "find did not see promised file"
ls -la "$visible_path" "$visible_path/docs" > "$work_dir/ls.out"
stat -c '%F %s %a' "$visible_path" "$visible_path/docs" "$file_path" > "$work_dir/stat.out"
grep -q '^regular file 24 644$' "$work_dir/stat.out" || fail "stat did not report promised file metadata"

if [ -s "$read_log" ]; then
    fail "metadata-only operations triggered provider reads"
fi

dd_output=$(dd if="$file_path" bs=1 skip=6 count=4 status=none)
[ "$dd_output" = "from" ] || fail "offset dd returned unexpected bytes: $dd_output"
grep -q '^READ offset=6 ' "$read_log" || fail "offset dd did not request offset 6"
if grep -Eq '^READ offset=[0-5] ' "$read_log"; then
    fail "offset dd requested bytes before requested offset"
fi

cat_output=$(cat "$file_path")
[ "$cat_output" = "hello from fuse-promise" ] || fail "cat returned unexpected bytes"
cp "$file_path" "$copy_file"
cmp "$expected_file" "$copy_file" >/dev/null || fail "cp output did not match provider data"

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

if cat "$file_path" > "$work_dir/after-disconnect.out" 2> "$work_dir/after-disconnect.err"; then
    fail "read after provider disconnect unexpectedly succeeded"
fi

echo "read-only MVP smoke passed"
