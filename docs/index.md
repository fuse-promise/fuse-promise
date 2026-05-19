# fuse-promise

`fuse-promise` is a Linux user-space Promise filesystem runtime built on
FUSE. It lets a provider publish a filesystem tree before file contents exist
locally. Metadata is visible immediately through ordinary paths, while file
bytes are supplied on demand when a process reads the file or when content is
materialized into local storage.

This repository is a system component. It owns the public C ABI, runtime,
daemon, FUSE adapter, metadata model, provider session model, cache policy, and
materialize operation.

It is not a storage provider, clipboard tool, desktop integration, cloud
client, or transport layer.

## Public Boundary

Applications integrate through the stable C ABI:

```c
#include <fuse-promise/fuse-promise.h>
```

Link with:

```sh
pkg-config --cflags --libs fuse-promise
```

The daemon IPC is private and is not a supported API.

## FUSE Backends

Release packages are built for both userspace FUSE backends:

| Package | Backend | Runtime dependency |
|---|---|---|
| `fuse-promise` | FUSE2 | `fuse`, `libfuse2`, `fusermount` |
| `fuse3-promise` | FUSE3 | `fuse3`, `libfuse3`, `fusermount3` |

Both packages install the same public commands and library names:

```text
fuse-promised
fpctl
libfusepromise.so.1
fuse-promise/fuse-promise.h
```

## Start Here

- [Public API](public-api.md) shows the normal provider flow and documents each
  public function with a usage example.
- [Packaging](packaging.md) documents the DEB/RPM package names, FUSE2/FUSE3
  variants, and release workflow.
- [Testing](testing.md) documents local build gates and mounted FUSE smoke
  tests.
- [Maintenance](maintenance.md) documents how this repository's docs, public
  ABI, packages, and releases are maintained.
