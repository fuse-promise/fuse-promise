# Packaging

`fuse-promise` packages as a Linux user-session system component. A downstream
package should install the public C ABI, the shared library, the daemon, the
CLI, pkg-config metadata, and the systemd user service.

## Build Inputs

Required build inputs:

- Rust toolchain matching the workspace `rust-version`.
- C compiler for ABI examples and downstream provider builds.
- `pkg-config`.
- libfuse3 development files when building the daemon with the `fuse-mount`
  feature.

Runtime inputs for the FUSE-enabled daemon:

- Linux FUSE support.
- `/dev/fuse`.
- `fusermount3`.
- libfuse3 runtime library.
- A valid user-owned `XDG_RUNTIME_DIR`.

## Build

Default workspace builds do not require libfuse3:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

Distribution builds should enable the daemon FUSE mount feature:

```sh
cargo build -p fuse-promise-daemon --features fuse-mount --locked
```

## Install Layout

The developer install script installs the expected distribution payload:

```sh
PREFIX=/usr DAEMON_FEATURES=fuse-mount scripts/install-dev.sh
```

Installed files:

```text
<includedir>/fuse-promise/fuse-promise.h
<libdir>/libfusepromise.so.<version>
<libdir>/libfusepromise.so.0
<libdir>/libfusepromise.so
<libdir>/pkgconfig/fuse-promise.pc
<bindir>/fuse-promised
<bindir>/fpctl
<prefix>/lib/systemd/user/fuse-promised.service
```

The installed public boundary is:

```sh
pkg-config --cflags --libs fuse-promise
```

## DESTDIR Staging

Packagers should stage files with `DESTDIR` while keeping installed metadata
paths rooted at the final prefix:

```sh
DESTDIR="$pkgdir" PREFIX=/usr DAEMON_FEATURES=fuse-mount scripts/install-dev.sh
```

The generated pkg-config file and systemd service must not include the staging
root in `includedir`, `libdir`, or `ExecStart`.

## User Service

The service is a systemd user unit and should be installed under the
distribution's user service directory, usually:

```text
/usr/lib/systemd/user/fuse-promised.service
```

The service starts:

```text
fuse-promised --foreground
```

The daemon remains user-session scoped and mounts under:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

## Verification

Run the install metadata gate from a clean tree:

```sh
tests/install-metadata.sh
```

For FUSE-enabled packages, also run the mounted smoke gate in an environment
with libfuse3 development metadata, `/dev/fuse`, and `fusermount3`:

```sh
tests/read-only-mvp-smoke.sh
```
