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

expected_pattern() {
    local offset=$1
    local count=$2
    local pattern=0123456789abcdef
    local output=
    local index
    local pattern_index
    for ((index = 0; index < count; index++)); do
        pattern_index=$(((offset + index) % ${#pattern}))
        output+="${pattern:pattern_index:1}"
    done
    printf '%s' "$output"
}

read_range() {
    local path=$1
    local offset=$2
    local count=$3
    local actual
    local expected
    if ((offset % count != 0)); then
        fail "test bug: offset $offset is not divisible by count $count"
    fi
    actual=$(dd if="$path" bs="$count" skip="$((offset / count))" count=1 status=none)
    expected=$(expected_pattern "$offset" "$count")
    [ "$actual" = "$expected" ] \
        || fail "random read at offset $offset returned unexpected bytes"
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
provider_bin="$work_dir/performance-stress-provider"
provider_lib_dir="$work_dir/lib"
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
: > "$read_log"

cd "$repo_dir"
cargo build -p fuse-promise-ffi --locked
cargo build -p fpctl --locked
cargo build -p fuse-promise-daemon --features fuse-mount --locked

ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so"
ln -s "$repo_dir/target/debug/libfusepromise.so" "$provider_lib_dir/libfusepromise.so.0"
cc -std=c11 -Wall -Wextra -Werror -I"$repo_dir/include" \
    "$repo_dir/tests/performance_stress_provider.c" \
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
    XDG_RUNTIME_DIR="$runtime_dir" "$provider_bin" "$read_log" \
    > "$provider_out" 2> "$provider_err" &
provider_pid=$!

visible_path=
tree_files=
large_file_size=
for _ in $(seq 1 100); do
    visible_path=$(sed -n 's/^visible_path=//p' "$provider_out" | tail -n 1)
    tree_files=$(sed -n 's/^tree_files=//p' "$provider_out" | tail -n 1)
    large_file_size=$(sed -n 's/^large_file_size=//p' "$provider_out" | tail -n 1)
    if [ -n "$visible_path" ] && [ -n "$tree_files" ] && [ -n "$large_file_size" ]; then
        break
    fi
    kill -0 "$provider_pid" 2>/dev/null || fail "provider exited before commit"
    sleep 0.1
done
[ -n "$visible_path" ] || fail "provider did not print visible path"
[ "$tree_files" = "300" ] || fail "provider reported unexpected tree file count: $tree_files"
[ "$large_file_size" = "262144" ] || fail "provider reported unexpected large file size: $large_file_size"

tree_path="$visible_path/tree"
large_path="$visible_path/large.bin"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" status \
    > "$work_dir/status.out"
grep -q '^cache_policy=no-cache$' "$work_dir/status.out" \
    || fail "stress daemon did not use default no-cache policy"

find "$tree_path" -type f -printf '%P\n' | sort > "$work_dir/tree-files.out"
actual_tree_files=$(wc -l < "$work_dir/tree-files.out")
[ "$actual_tree_files" = "$tree_files" ] \
    || fail "large tree file count mismatch: $actual_tree_files"
stat -c '%F %s %a' "$large_path" > "$work_dir/large-stat.out"
grep -q "^regular file $large_file_size 644$" "$work_dir/large-stat.out" \
    || fail "large file metadata mismatch"
if [ -s "$read_log" ]; then
    fail "large tree metadata operations triggered provider reads"
fi

read_range "$large_path" 0 32
read_range "$large_path" 4096 32
read_range "$large_path" 65504 32
read_range "$large_path" 131040 32
read_range "$large_path" 262112 32

for offset in 0 4096 65504 131040 262112; do
    grep -q "^READ path=large.bin offset=$offset " "$read_log" \
        || fail "missing random read at offset $offset"
done
if grep -q "^READ path=large.bin offset=0 length=$large_file_size$" "$read_log"; then
    fail "random reads triggered full-file provider read"
fi
total_provider_bytes=$(
    awk '/^READ path=large.bin / {
        for (field_index = 1; field_index <= NF; field_index++) {
            if ($field_index ~ /^length=/) {
                sub("length=", "", $field_index);
                sum += $field_index;
            }
        }
    } END { print sum + 0 }' "$read_log"
)
[ "$total_provider_bytes" -lt 4096 ] \
    || fail "random reads transferred too much provider data: $total_provider_bytes"

echo "Performance stress passed"
