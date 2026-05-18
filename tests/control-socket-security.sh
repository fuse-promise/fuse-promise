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
    if [ -f "${daemon_out:-}" ]; then
        echo "--- daemon.out ---" >&2
        cat "$daemon_out" >&2
    fi
    if [ -f "${daemon_err:-}" ]; then
        echo "--- daemon.err ---" >&2
        cat "$daemon_err" >&2
    fi
    exit 1
}

command -v cargo >/dev/null || skip "cargo is required"

work_dir=$(mktemp -d)
runtime_dir="$work_dir/runtime"
daemon_out="$work_dir/daemon.out"
daemon_err="$work_dir/daemon.err"

cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT

mkdir -m 700 "$runtime_dir"
printf 'not a socket\n' > "$runtime_dir/fuse-promise.sock"

cd "$repo_dir"
cargo build -p fuse-promise-daemon --locked

if XDG_RUNTIME_DIR="$runtime_dir" "$repo_dir/target/debug/fuse-promised" \
    --foreground > "$daemon_out" 2> "$daemon_err"; then
    fail "daemon accepted a non-socket control path"
fi

grep -q "control socket path exists and is not a socket" "$daemon_err" \
    || fail "daemon did not report the non-socket control path"

echo "Control socket security passed"
