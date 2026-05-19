# Packaging

`fuse-promise` packages as a Linux user-session system component. A downstream
package should install the public C ABI, the shared library, the daemon, the
CLI, pkg-config metadata, and the systemd user service.

## Build Inputs

Required build inputs:

- Rust toolchain matching the workspace `rust-version`.
- C compiler for ABI examples and downstream provider builds.
- `pkg-config`.
- libfuse3 development files for the `fuse-mount-fuse3` daemon feature.
- libfuse2 development files for the `fuse-mount-fuse` daemon feature.

Runtime inputs for the FUSE-enabled daemon:

- Linux FUSE support.
- `/dev/fuse`.
- `fusermount3` and libfuse3 runtime library for FUSE3 packages.
- `fusermount` and libfuse2 runtime library for FUSE2 packages.
- A valid user-owned `XDG_RUNTIME_DIR`.

## Build

Default workspace builds do not require libfuse development files:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

Distribution builds should enable the daemon FUSE mount feature:

```sh
cargo build -p fuse-promise-daemon --features fuse-mount --locked
cargo build -p fuse-promise-daemon --features fuse-mount-fuse --locked
cargo build -p fuse-promise-daemon --features fuse-mount-fuse3 --locked
```

## Install Layout

The developer install script installs the expected distribution payload:

```sh
PREFIX=/usr DAEMON_FEATURES=fuse-mount scripts/install-dev.sh
PREFIX=/usr DAEMON_FEATURES=fuse-mount-fuse scripts/install-dev.sh
PREFIX=/usr DAEMON_FEATURES=fuse-mount-fuse3 scripts/install-dev.sh
```

Installed files:

```text
<includedir>/fuse-promise/fuse-promise.h
<libdir>/libfusepromise.so.<version>
<libdir>/libfusepromise.so.1
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
DESTDIR="$pkgdir" PREFIX=/usr DAEMON_FEATURES=fuse-mount-fuse3 scripts/install-dev.sh
```

The generated pkg-config file and systemd service must not include the staging
root in `includedir`, `libdir`, or `ExecStart`.

## DEB and RPM Packages

The repository ships an nFPM configuration and a local package wrapper:

```sh
FUSE_PROMISE_FUSE_BACKEND=fuse3 scripts/package-linux.sh
FUSE_PROMISE_FUSE_BACKEND=fuse DIST_DIR=dist/fuse scripts/package-linux.sh
```

The wrapper stages a release build with:

```sh
DESTDIR=<stage> PREFIX=/usr BUILD_PROFILE=release SONAME_MAJOR=1 DAEMON_FEATURES=<backend-feature> scripts/install-dev.sh
```

It then writes these artifacts to `dist/`:

```text
fuse3-promise_<version>-1_<arch>.deb
fuse3-promise-<version>-1.<arch>.rpm
fuse-promise_<version>-1_<arch>.deb
fuse-promise-<version>-1.<arch>.rpm
SHA256SUMS
```

The generated packages contain the public C ABI, shared library symlinks for
SONAME major `1`, pkg-config metadata, `fuse-promised`, `fpctl`, and the
systemd user service.

Release builds produce native Linux packages for the main CPU architectures:

```text
fuse3-promise_<version>-1_amd64.deb
fuse3-promise_<version>-1_arm64.deb
fuse3-promise-<version>-1.x86_64.rpm
fuse3-promise-<version>-1.aarch64.rpm
fuse-promise_<version>-1_amd64.deb
fuse-promise_<version>-1_arm64.deb
fuse-promise-<version>-1.x86_64.rpm
fuse-promise-<version>-1.aarch64.rpm
fuse-promise-<version>.tar.gz
SHA256SUMS
```

The FUSE2 and FUSE3 packages install the same executable, library, header, and
service paths. They are separate package names and conflict with each other.

Architecture names differ by package family: Debian uses `amd64` and `arm64`,
while RPM uses `x86_64` and `aarch64`.

Distribution names such as Ubuntu Jammy, Ubuntu Noble, Debian Bookworm, EL 9,
or Fedora are repository metadata targets. They do not always require separate
binary builds. Build separate distribution packages only when dependency names,
library ABI, or service layout differ.

## GitHub Actions

The repository uses custom workflows instead of the generic GitHub Rust
template:

- `CI` runs deterministic Rust, ABI, install metadata, and security gates on
  GitHub-hosted Ubuntu runners.
- `FUSE Stable Gates` runs `tests/stable-release-gates.sh` on a self-hosted
  runner labeled `linux` and `fuse`, because mounted FUSE tests require
  `/dev/fuse` and a matching `fusermount` helper.
- `Release` validates the tag, builds FUSE2 and FUSE3 DEB/RPM artifacts for
  `amd64` and `arm64`, builds a source tarball, uploads them to the GitHub
  Release, and optionally publishes packages to Cloudsmith.
- Cloudsmith repository publishing is gated on the mounted FUSE tests passing.
  If a GitHub-hosted runner lacks `/dev/fuse`, the workflow can still build
  GitHub Release assets, but public apt/yum repository publishing requires a
  runner with FUSE support.

Set the optional `RELEASE_VALIDATION_RUNNER` repository variable to choose the
runner for release validation. The default is `"ubuntu-22.04"`. To publish
through a self-hosted FUSE runner, set it to JSON:

```json
["self-hosted", "linux", "fuse"]
```

To publish public apt/yum repositories through Cloudsmith, configure:

```text
CLOUDSMITH_API_KEY        repository secret
CLOUDSMITH_REPOSITORY     repository variable, for example owner/repository
```

Optional repository variables select upload targets:

```text
CLOUDSMITH_DEB_DISTRIBUTION    default ubuntu
CLOUDSMITH_DEB_RELEASE         default any-version
CLOUDSMITH_DEB_COMPONENT       default main
CLOUDSMITH_RPM_DISTRIBUTION    default el
CLOUDSMITH_RPM_RELEASE         default 9
```

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
with the selected libfuse development metadata, `/dev/fuse`, and the matching
`fusermount` helper:

```sh
tests/read-only-mvp-smoke.sh
```
