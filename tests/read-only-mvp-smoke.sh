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
expected_tree="$work_dir/expected-tree"
copy_file="$work_dir/copied.txt"
materialize_dir="$work_dir/materialized"
materialize_tree_dir="$work_dir/materialized-tree"
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
mkdir "$materialize_dir"
mkdir -p "$expected_tree/docs/guides" "$expected_tree/docs/empty" "$materialize_tree_dir"
printf 'hello from fuse-promise\n' > "$expected_file"
cp "$expected_file" "$expected_tree/docs/readme.txt"
printf 'setup guide\n' > "$expected_tree/docs/guides/setup.txt"
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
docs_path="$visible_path/docs"
setup_path="$visible_path/docs/guides/setup.txt"
pending_path="$visible_path/pending.txt"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" status \
    > "$work_dir/status.out"
grep -q '^mount=mounted$' "$work_dir/status.out" \
    || fail "fpctl status did not report mounted"
grep -q '^cache_policy=no-cache$' "$work_dir/status.out" \
    || fail "fpctl status did not report no-cache policy"
XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" list \
    | grep -q '^promises=1$' || fail "fpctl list did not report one promise"

find "$visible_path" -maxdepth 3 -printf '%y %P\n' | sort > "$work_dir/find.out"
grep -q '^d docs$' "$work_dir/find.out" || fail "find did not see docs directory"
grep -q '^d docs/empty$' "$work_dir/find.out" || fail "find did not see empty directory"
grep -q '^d docs/guides$' "$work_dir/find.out" || fail "find did not see nested directory"
grep -q '^f docs/readme.txt$' "$work_dir/find.out" || fail "find did not see promised file"
grep -q '^f docs/guides/setup.txt$' "$work_dir/find.out" || fail "find did not see nested promised file"
grep -q '^f pending.txt$' "$work_dir/find.out" || fail "find did not see unmaterialized promised file"
ls -la "$visible_path" "$visible_path/docs" > "$work_dir/ls.out"
stat -c '%F %s %a' "$visible_path" "$visible_path/docs" "$file_path" "$setup_path" "$pending_path" > "$work_dir/stat.out"
grep -q '^regular file 24 644$' "$work_dir/stat.out" || fail "stat did not report promised file metadata"
grep -q '^regular file 12 644$' "$work_dir/stat.out" || fail "stat did not report nested promised file metadata"
grep -q '^regular file 13 644$' "$work_dir/stat.out" || fail "stat did not report unmaterialized promised file metadata"

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

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" \
    materialize "$file_path" "$materialize_dir" > "$work_dir/materialize.out"
grep -q "^target_path=$materialize_dir/readme.txt$" "$work_dir/materialize.out" \
    || fail "materialize did not report expected target path"
grep -q '^bytes_written=24$' "$work_dir/materialize.out" \
    || fail "materialize did not report expected byte count"
cmp "$expected_file" "$materialize_dir/readme.txt" >/dev/null \
    || fail "materialized file did not match provider data"
materialized_stat=$(stat -c '%s %a %Y' "$materialize_dir/readme.txt")
[ "$materialized_stat" = "24 644 0" ] \
    || fail "materialized metadata mismatch: $materialized_stat"
if XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" \
    materialize "$file_path" "$materialize_dir" > "$work_dir/materialize-conflict.out" 2> "$work_dir/materialize-conflict.err"; then
    fail "materialize conflict unexpectedly succeeded"
fi
grep -q "already exists" "$work_dir/materialize-conflict.err" \
    || fail "materialize conflict did not report already exists"

XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fpctl" \
    materialize "$docs_path" "$materialize_tree_dir" > "$work_dir/materialize-tree.out"
grep -q "^target_path=$materialize_tree_dir/docs$" "$work_dir/materialize-tree.out" \
    || fail "directory materialize did not report expected target path"
grep -q '^bytes_written=36$' "$work_dir/materialize-tree.out" \
    || fail "directory materialize did not report expected byte count"
grep -q '^files_written=2$' "$work_dir/materialize-tree.out" \
    || fail "directory materialize did not report expected file count"
grep -q '^directories_created=3$' "$work_dir/materialize-tree.out" \
    || fail "directory materialize did not report expected directory count"
diff -r "$expected_tree/docs" "$materialize_tree_dir/docs" >/dev/null \
    || fail "directory materialize did not match expected tree"
directory_stat=$(stat -c '%a %Y' "$materialize_tree_dir/docs" "$materialize_tree_dir/docs/guides" "$materialize_tree_dir/docs/empty")
[ "$directory_stat" = "$(printf '755 0\n755 0\n755 0')" ] \
    || fail "directory materialize metadata mismatch: $directory_stat"

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

cat "$file_path" > "$work_dir/after-disconnect-materialized.out" \
    || fail "materialized read after provider disconnect failed"
cmp "$expected_file" "$work_dir/after-disconnect-materialized.out" >/dev/null \
    || fail "materialized read after provider disconnect returned unexpected data"

if cat "$pending_path" > "$work_dir/after-disconnect-unmaterialized.out" 2> "$work_dir/after-disconnect-unmaterialized.err"; then
    fail "unmaterialized read after provider disconnect unexpectedly succeeded"
fi

echo "MVP smoke passed"
