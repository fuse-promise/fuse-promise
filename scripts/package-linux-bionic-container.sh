#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v docker >/dev/null || fail "docker is required for Ubuntu 18.04 compatibility packaging"

image=${FUSE_PROMISE_PACKAGE_IMAGE:-ubuntu:18.04}
rust_toolchain=${RUST_TOOLCHAIN:-1.85}
go_toolchain=${GO_TOOLCHAIN:-1.25.0}
nfpm_version=${NFPM_VERSION:-v2.46.3}

dist_dir_host=$(realpath -m "${DIST_DIR:-"$repo_dir/dist"}")
repo_abs=$(realpath -m "$repo_dir")
case "$dist_dir_host" in
    "$repo_abs"/*) container_dist_dir="/work/${dist_dir_host#"$repo_abs"/}" ;;
    *) fail "DIST_DIR must be inside the repository when using container packaging: $dist_dir_host" ;;
esac

mkdir -p "$dist_dir_host"

docker run --rm \
    -e DEBIAN_FRONTEND=noninteractive \
    -e RUST_TOOLCHAIN="$rust_toolchain" \
    -e GO_TOOLCHAIN="$go_toolchain" \
    -e NFPM_VERSION="$nfpm_version" \
    -e FUSE_PROMISE_FUSE_BACKEND="${FUSE_PROMISE_FUSE_BACKEND:-fuse3}" \
    -e FUSE_PROMISE_ARCH="${FUSE_PROMISE_ARCH:-}" \
    -e FUSE_PROMISE_RPM_ARCH="${FUSE_PROMISE_RPM_ARCH:-}" \
    -e FUSE_PROMISE_MAX_GLIBC="${FUSE_PROMISE_MAX_GLIBC:-2.27}" \
    -e DIST_DIR="$container_dist_dir" \
    -e HOST_UID="$(id -u)" \
    -e HOST_GID="$(id -g)" \
    -v "$repo_abs:/work" \
    -w /work \
    "$image" \
    bash -lc '
        set -euo pipefail

        if ! apt-get update; then
            sed -i \
                -e "s|http://archive.ubuntu.com/ubuntu|http://old-releases.ubuntu.com/ubuntu|g" \
                -e "s|http://security.ubuntu.com/ubuntu|http://old-releases.ubuntu.com/ubuntu|g" \
                /etc/apt/sources.list
            apt-get update
        fi
        apt-get install -y --no-install-recommends \
            build-essential \
            ca-certificates \
            curl \
            file \
            fuse3 \
            git \
            gzip \
            libfuse-dev \
            libfuse3-dev \
            pkg-config \
            rpm \
            tar \
            xz-utils
        update-ca-certificates

        git config --global --add safe.directory /work

        curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN"
        export PATH="$HOME/.cargo/bin:$PATH"

        case "$(uname -m)" in
            x86_64) go_arch=amd64 ;;
            aarch64 | arm64) go_arch=arm64 ;;
            *) echo "unsupported Go host architecture: $(uname -m)" >&2; exit 1 ;;
        esac
        curl -fsSL "https://go.dev/dl/go${GO_TOOLCHAIN}.linux-${go_arch}.tar.gz" \
            -o /tmp/go.tar.gz
        rm -rf /usr/local/go
        tar -C /usr/local -xzf /tmp/go.tar.gz
        export PATH="/usr/local/go/bin:$HOME/go/bin:$PATH"

        go install "github.com/goreleaser/nfpm/v2/cmd/nfpm@$NFPM_VERSION"

        bash scripts/package-linux.sh

        if [ -n "${HOST_UID:-}" ] && [ -n "${HOST_GID:-}" ]; then
            chown -R "$HOST_UID:$HOST_GID" "$DIST_DIR"
            [ ! -d target ] || chown -R "$HOST_UID:$HOST_GID" target
        fi
    '
