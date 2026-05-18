#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v git >/dev/null || fail "git is required"
command -v gzip >/dev/null || fail "gzip is required"
command -v sha256sum >/dev/null || fail "sha256sum is required"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -n 1)
[ -n "$version" ] || fail "could not read workspace version"

repo_abs=$(realpath -m "$repo_dir")
dist_dir=$(realpath -m "${DIST_DIR:-"$repo_dir/dist"}")
if [ "$dist_dir" = "/" ] || [ "$dist_dir" = "$repo_abs" ]; then
    fail "refusing unsafe DIST_DIR: $dist_dir"
fi
case "$repo_abs/" in
    "$dist_dir"/*) fail "DIST_DIR must not contain the repository root: $dist_dir" ;;
esac

source_ref=${SOURCE_REF:-HEAD}
archive="$dist_dir/fuse-promise-$version.tar.gz"

mkdir -p "$dist_dir"

cd "$repo_dir"
git rev-parse "$source_ref^{commit}" >/dev/null
git archive --worktree-attributes --format=tar --prefix="fuse-promise-$version/" "$source_ref" \
    | gzip -n > "$archive"

(
    cd "$dist_dir"
    rm -f SHA256SUMS
    shopt -s nullglob
    artifacts=(*.deb *.rpm *.tar.gz)
    [ "${#artifacts[@]}" -gt 0 ] || fail "no release artifacts were generated"
    sha256sum "${artifacts[@]}" > SHA256SUMS
)

ls -la "$dist_dir"
