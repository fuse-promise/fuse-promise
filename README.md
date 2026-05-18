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

## Minimal Provider Example

A provider publishes metadata first, then supplies file bytes when the kernel
reads a promised file. Writes into local storage are done through materialize;
mounted write callbacks are not part of the current public ABI.

```c
static const char kData[] = "hello from fuse-promise example\n";

static fp_status_t read_file(const fp_read_request_t *request,
                             fp_read_response_t *response,
                             void *user_data) {
    (void)user_data;
    if (strcmp(request->node_id, "example-file") != 0 ||
        strcmp(request->relative_path, "docs/hello.txt") != 0) {
        return FP_ERR_NOT_FOUND;
    }

    size_t size = strlen(kData);
    if (request->offset >= size) {
        response->bytes_written = 0;
        return FP_OK;
    }

    size_t available = size - (size_t)request->offset;
    size_t count = request->length < available ? request->length : available;
    if (count > response->buffer_len) {
        count = response->buffer_len;
    }

    memcpy(response->buffer, kData + request->offset, count);
    response->bytes_written = count;
    return FP_OK;
}

fp_context_options_t options = FP_CONTEXT_OPTIONS_INIT;
options.runtime_dir = runtime_dir;
fp_context_open(&options, &context);

fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_file);
fp_provider_register(context, &ops, NULL, &provider);

fp_promise_builder_new(context, provider, &builder);

fp_node_attr_t dir_attr = FP_NODE_ATTR_INIT;
dir_attr.mode = 0755;
dir_attr.mtime_nsec = 1700000000000000000LL;
fp_promise_add_dir(builder, "docs", &dir_attr, "example-dir");

fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
file_attr.mode = 0644;
file_attr.size = strlen(kData);
file_attr.mtime_nsec = 1700000000000000000LL;
fp_promise_add_file(builder, "docs/hello.txt", &file_attr, "example-file");

char visible_path[4096];
fp_promise_commit(builder, visible_path, sizeof(visible_path));
```

The complete buildable source is
[examples/minimal_provider.c](examples/minimal_provider.c). It covers directory
and file attributes, provider-backed reads, and materialize into a local target.

Run it locally:

```sh
prefix="$PWD/.local"
PREFIX="$prefix" DAEMON_FEATURES=fuse-mount scripts/install-dev.sh
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"

cc -std=c11 -Wall -Wextra -Werror examples/minimal_provider.c \
  $(pkg-config --cflags --libs fuse-promise) \
  "-Wl,-rpath,$prefix/lib" \
  -o /tmp/minimal_provider

runtime=$(mktemp -d)
XDG_RUNTIME_DIR="$runtime" "$prefix/bin/fuse-promised" --foreground &
/tmp/minimal_provider "$runtime" &

cat "$runtime/fuse-promise/promise-1/docs/hello.txt"
mkdir "$runtime/out"
XDG_RUNTIME_DIR="$runtime" "$prefix/bin/fpctl" materialize \
  "$runtime/fuse-promise/promise-1/docs/hello.txt" "$runtime/out"
cat "$runtime/out/hello.txt"
```

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

Minimal mounted smoke test:

```sh
tests/minimal-provider-smoke.sh
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
