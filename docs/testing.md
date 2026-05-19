# Testing

The default test path checks the Rust workspace and ABI-facing behavior without
requiring a mounted FUSE filesystem.

```sh
cargo check --workspace --locked
cargo test --workspace --locked
```

## Backend Build Checks

The daemon can be compiled against either libfuse backend:

```sh
cargo check -p fuse-promise-daemon --features fuse-mount-fuse --locked
cargo check -p fuse-promise-daemon --features fuse-mount-fuse3 --locked
```

Use these checks to verify that a FUSE2 build does not require the FUSE3 daemon
feature, and that a FUSE3 build does not require the FUSE2 daemon feature.

## Mounted Smoke Tests

Mounted tests require Linux FUSE support, `/dev/fuse`, the matching
`fusermount` helper, and the matching libfuse development metadata.

Debian and Ubuntu setup:

```sh
sudo apt-get install build-essential pkg-config libfuse-dev libfuse3-dev fuse3
```

Minimal provider smoke tests:

```sh
FUSE_PROMISE_FUSE_BACKEND=fuse3 tests/minimal-provider-smoke.sh
FUSE_PROMISE_FUSE_BACKEND=fuse tests/minimal-provider-smoke.sh
```

The minimal provider test builds `examples/minimal_provider.c`, starts
`fuse-promised`, commits a Promise tree with directory and file metadata, reads
provider-backed file bytes through the mount, and materializes the promised file
into local storage with `fpctl`.

## Stable Release Gate

The release gate checks formatting, build metadata, ABI hardening, security
behavior, and mounted FUSE behavior where available:

```sh
BUILD_PROFILE=release SONAME_MAJOR=1 tests/stable-release-gates.sh
```

GitHub-hosted runners may not expose `/dev/fuse`. The repository keeps mounted
FUSE gates separate so they can run on a self-hosted Linux runner with FUSE
enabled.
