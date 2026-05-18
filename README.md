# fuse-promise

Linux user-space Promise filesystem runtime built on FUSE3.

`fuse-promise` lets a provider publish a filesystem tree before file contents
exist locally. Metadata is visible immediately through ordinary paths. File
bytes are supplied on demand when a process reads the file, or written into
local storage through materialize.

This repository is a system component. It is not a storage provider, clipboard
tool, desktop integration, cloud client, or transport layer.

## Why This Exists

Windows has platform APIs for this pattern:
[Cloud Files API](https://learn.microsoft.com/windows/win32/cfapi/cloud-files-api-portal)
for placeholder files, and
[Projected File System](https://learn.microsoft.com/windows/win32/projfs/projected-file-system)
for user-mode providers that project trees into the filesystem. macOS has
[File Provider](https://developer.apple.com/documentation/fileprovider)
and dataless files, where metadata can exist locally before content is
materialized.

Linux has FUSE, but not a common Promise-file runtime with a stable C ABI.
`fuse-promise` provides that lower layer for Linux.

## Interface

The public interface is the C ABI:

```c
#include <fuse-promise/fuse-promise.h>
```

Consumers link:

```sh
pkg-config --cflags --libs fuse-promise
```

Installed public surface:

```text
/usr/include/fuse-promise/fuse-promise.h
/usr/lib/libfusepromise.so.1
/usr/lib/libfusepromise.so
/usr/lib/pkgconfig/fuse-promise.pc
/usr/bin/fuse-promised
/usr/bin/fpctl
/usr/lib/systemd/user/fuse-promised.service
```

Daemon IPC is private and is not a supported API.

## Runtime Requirements

Default user-session mount:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

Required runtime dependencies for mounted operation:

```text
Linux FUSE kernel support
/dev/fuse
fuse3
libfuse3
fusermount3
```

Packaged builds target Ubuntu 22.04 or newer.

## Build and Test

Default workspace build:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
```

FUSE-enabled daemon build:

```sh
cargo build -p fuse-promise-daemon --features fuse-mount --locked
```

Required system packages on Debian/Ubuntu:

```sh
sudo apt-get install build-essential pkg-config libfuse3-dev fuse3
```

Release gate:

```sh
BUILD_PROFILE=release SONAME_MAJOR=1 tests/stable-release-gates.sh
```

The full gate requires `/dev/fuse`, `fusermount3`, and libfuse3 development
metadata.

## Install and Package

Developer install into `/usr/local`:

```sh
scripts/install-dev.sh
```

Distribution-style staging:

```sh
DESTDIR="$pkgdir" PREFIX=/usr BUILD_PROFILE=release SONAME_MAJOR=1 DAEMON_FEATURES=fuse-mount scripts/install-dev.sh
```

Release packaging uses nFPM:

```sh
scripts/package-linux.sh
```

Release artifacts:

```text
fuse-promise_<version>-1_amd64.deb
fuse-promise_<version>-1_arm64.deb
fuse-promise-<version>-1.x86_64.rpm
fuse-promise-<version>-1.aarch64.rpm
fuse-promise-<version>.tar.gz
SHA256SUMS
```

## Source Layout

```text
include/fuse-promise/        public C ABI
crates/fuse-promise-ffi/     libfusepromise implementation
crates/fuse-promise-daemon/  fuse-promised daemon
crates/fuse-promise-runtime/ core runtime model
crates/fuse-promise-ipc/     private daemon IPC
tools/fpctl/                 administrative CLI
tests/                       release gates
packaging/                   package metadata
docs/                        design documents
```

## Documentation

- [Architecture](docs/architecture.md)
- [Public API](docs/public-api.md)
- [Packaging](docs/packaging.md)
- [Security](docs/security.md)
- [Changelog](CHANGELOG.md)
