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
libfuse3_version=${FUSE_PROMISE_LIBFUSE3_VERSION:-3.18.2}
libfuse3_url=${FUSE_PROMISE_LIBFUSE3_SOURCE_URL:-"https://github.com/libfuse/libfuse/releases/download/fuse-$libfuse3_version/fuse-$libfuse3_version.tar.gz"}
libfuse3_sha256=${FUSE_PROMISE_LIBFUSE3_SOURCE_SHA256:-f01de85717e20adf5f98aff324acd85dd73d61a5ca3834d573dcf0bd6e54a298}

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
    -e FUSE_PROMISE_LIBFUSE3_SOURCE_URL="$libfuse3_url" \
    -e FUSE_PROMISE_LIBFUSE3_SOURCE_SHA256="$libfuse3_sha256" \
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
        apt_packages=(
            build-essential \
            ca-certificates \
            curl \
            file \
            git \
            gzip \
            libfuse-dev \
            pkg-config \
            rpm \
            tar \
            xz-utils
        )
        if [ "$FUSE_PROMISE_FUSE_BACKEND" = "fuse3" ]; then
            apt_packages+=(ninja-build python3 python3-pip)
        fi

        apt-get install -y --no-install-recommends "${apt_packages[@]}"
        update-ca-certificates

        git config --global --add safe.directory /work

        if [ "$FUSE_PROMISE_FUSE_BACKEND" = "fuse3" ]; then
            python3 -m pip install --user --upgrade "pip<22"
            python3 -m pip install --user "meson==0.61.5"
            export PATH="$HOME/.local/bin:$PATH"

            curl -fsSL "$FUSE_PROMISE_LIBFUSE3_SOURCE_URL" -o /tmp/fuse3.tar.gz
            echo "$FUSE_PROMISE_LIBFUSE3_SOURCE_SHA256  /tmp/fuse3.tar.gz" | sha256sum -c -
            mkdir -p /tmp/fuse3-src
            tar -xzf /tmp/fuse3.tar.gz -C /tmp/fuse3-src --strip-components=1
            meson setup /tmp/fuse3-build /tmp/fuse3-src \
                --prefix=/usr/local \
                --libdir=lib \
                -Dexamples=false \
                -Dutils=false \
                -Dtests=false \
                -Denable-io-uring=false
            ninja -C /tmp/fuse3-build
            ninja -C /tmp/fuse3-build install

            export PKG_CONFIG_PATH="/usr/local/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
            export LD_LIBRARY_PATH="/usr/local/lib:${LD_LIBRARY_PATH:-}"
        fi

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
