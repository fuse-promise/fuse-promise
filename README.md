# fuse-promise

`fuse-promise` is a Linux user-space Promise filesystem runtime built on FUSE.

It exposes promised files and directories from metadata first, fetches file content only when a process actually reads it, and provides a standard materialize operation that converts promised nodes into real local files.

This repository is intentionally a system component. It is not a clipboard application, a cloud sync client, a remote desktop tool, or a storage provider. Those products should use the public C ABI provided by `libfusepromise.so` and the public headers installed under `fuse-promise/`.

## Core Boundary

`fuse-promise` owns:

- The Promise file model.
- The user-session daemon.
- The FUSE filesystem implementation.
- The stable public C ABI.
- The materialize operation.
- Runtime lifecycle, cache, inode, and metadata management.

`fuse-promise` does not own:

- Clipboard synchronization.
- Desktop drag-and-drop adapters.
- Cloud provider integrations.
- P2P transport protocols.
- Application-specific remote file protocols.

Upper-layer software may build those features by calling the public API.

## Target Install Shape

```text
/usr/include/fuse-promise/fuse-promise.h
/usr/lib/libfusepromise.so
/usr/lib/pkgconfig/fuse-promise.pc
/usr/lib/systemd/user/fuse-promised.service
/run/user/$UID/fuse-promise/
```

## Documents

- [Project Statement](docs/project-statement.md)
- [Requirements](docs/requirements.md)
- [Architecture](docs/architecture.md)
- [Language and ABI](docs/language-and-abi.md)
- [Promise Model](docs/promise-model.md)
- [Public API](docs/public-api.md)
- [Runtime](docs/runtime.md)
- [Development Style](docs/development-style.md)
- [Roadmap](docs/roadmap.md)
