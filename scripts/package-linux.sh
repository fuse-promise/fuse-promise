#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v nfpm >/dev/null || fail "nfpm is required"
command -v sha256sum >/dev/null || fail "sha256sum is required"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -n 1)
[ -n "$version" ] || fail "could not read workspace version"

case "$(uname -m)" in
    x86_64) host_package_arch=amd64 ;;
    aarch64 | arm64) host_package_arch=arm64 ;;
    *) host_package_arch= ;;
esac

case "${FUSE_PROMISE_ARCH:-}" in
    "")
        [ -n "$host_package_arch" ] \
            || fail "unsupported host architecture $(uname -m); set FUSE_PROMISE_ARCH explicitly"
        package_arch=$host_package_arch
        ;;
    *)
        package_arch=$FUSE_PROMISE_ARCH
        ;;
esac

if [ -n "$host_package_arch" ] \
    && [ "$package_arch" != "$host_package_arch" ] \
    && [ "${FUSE_PROMISE_ALLOW_ARCH_OVERRIDE:-}" != 1 ]; then
    fail "cross-architecture packaging is not supported by this script; run on $package_arch or set FUSE_PROMISE_ALLOW_ARCH_OVERRIDE=1"
fi

case "${FUSE_PROMISE_RPM_ARCH:-}" in
    "")
        case "$package_arch" in
            amd64) rpm_arch=x86_64 ;;
            arm64) rpm_arch=aarch64 ;;
            *) rpm_arch=$package_arch ;;
        esac
        ;;
    *)
        rpm_arch=$FUSE_PROMISE_RPM_ARCH
        ;;
esac

repo_abs=$(realpath -m "$repo_dir")
dist_dir=$(realpath -m "${DIST_DIR:-"$repo_dir/dist"}")
if [ "$dist_dir" = "/" ] || [ "$dist_dir" = "$repo_abs" ]; then
    fail "refusing unsafe DIST_DIR: $dist_dir"
fi
case "$repo_abs/" in
    "$dist_dir"/*) fail "DIST_DIR must not contain the repository root: $dist_dir" ;;
esac

work_dir=$(mktemp -d)
stage="$work_dir/stage"

cleanup() {
    rm -rf "$work_dir"
}
trap cleanup EXIT

rm -rf "$dist_dir"
mkdir -p "$dist_dir"

cd "$repo_dir"

DESTDIR="$stage" \
    PREFIX=/usr \
    BUILD_PROFILE=release \
    SONAME_MAJOR=1 \
    DAEMON_FEATURES=fuse-mount \
    scripts/install-dev.sh

export FUSE_PROMISE_VERSION="$version"
export FUSE_PROMISE_ARCH="$package_arch"
export FUSE_PROMISE_SONAME_MAJOR=1
export FUSE_PROMISE_STAGE="$stage"

nfpm package \
    --config packaging/nfpm.yaml \
    --packager deb \
    --target "$dist_dir/fuse-promise_${version}-1_${package_arch}.deb"

nfpm package \
    --config packaging/nfpm.yaml \
    --packager rpm \
    --target "$dist_dir/fuse-promise-${version}-1.${rpm_arch}.rpm"

(
    cd "$dist_dir"
    rm -f SHA256SUMS
    shopt -s nullglob
    artifacts=(*.deb *.rpm)
    [ "${#artifacts[@]}" -gt 0 ] || fail "no package artifacts were generated"
    sha256sum "${artifacts[@]}" > SHA256SUMS
)

ls -la "$dist_dir"
