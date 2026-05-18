#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

export BUILD_PROFILE=${BUILD_PROFILE:-release}
export SONAME_MAJOR=${SONAME_MAJOR:-1}

cd "$repo_dir"

cargo fmt --check --all
cargo check --workspace --locked
cargo test --workspace --locked
tests/abi-hardening.sh
tests/install-metadata.sh
tests/read-only-mvp-smoke.sh
tests/read-through-cache-smoke.sh
tests/performance-stress.sh
tests/control-socket-security.sh
tests/materialize-security.sh
git diff --check

echo "stable release gates passed"
