# fuse-promise

Linux promised files, implemented as a user-space FUSE runtime.

`fuse-promise` lets a provider publish a filesystem tree before the file contents are present locally. Applications see ordinary paths. Metadata is available immediately. File data is supplied later, on demand, when a process reads the file. A promised file or directory can also be materialized into real local storage.

The project is a system component. It is not a clipboard application, a cloud client, a remote desktop tool, or a storage provider. Those systems can be built on top of `fuse-promise` through its public C ABI.

## Status

This repository is in the early implementation phase.

The current tree contains the public C header, Rust workspace skeleton, core
Promise metadata model, C ABI entry points, initial daemon and CLI entry
points, private framed status IPC used by `fpctl status`, private provider
register/unregister IPC messages, private Promise metadata commit IPC, and
private provider read request/response message helpers with connection-scoped
provider disconnect propagation. `fp_provider_register()` now registers with
the daemon through private IPC, and provider read requests received on that
connection are dispatched to the public C callback. The runtime can plan
provider-owned file reads with provider-gone and EOF handling, and the daemon
IPC state can route provider read requests over registered provider
connections. The daemon has a feature-gated FUSE mount lifecycle skeleton
behind the `fuse-mount` feature; default builds report `fuse_adapter=disabled`
until the libfuse3 development dependency is present. Read-only FUSE operations
and the materialize engine are still under development. Private metadata commit
is gated on mount readiness so unmounted daemon state cannot create invisible
promises. The public commit and materialize calls currently return
`FP_ERR_UNAVAILABLE` rather than claiming a visible FUSE path that does not
exist yet.

The first implementation target is a read-only Promise filesystem MVP:

- Commit metadata-only file and directory trees.
- Expose them under a user-session FUSE mount.
- Support `stat`, `readdir`, `open`, and offset-based `read`.
- Route reads to provider callbacks.

Materialization is the next phase after the read-only filesystem path is
working end to end.

## Why

Linux has strong support for user-space filesystems through FUSE, but it does not provide a common Promise file model.

Many systems need this model:

- Remote file handoff.
- Cross-device file transfer.
- Lazy cloud or workspace mounts.
- Large file workflows where copying metadata first is cheap.
- Applications that can declare files now and provide bytes later.

Without a shared layer, each application invents its own placeholder format, transport, lifecycle, and materialization behavior. `fuse-promise` defines that missing lower layer as a reusable Linux component.

## Model

A promised file is a regular-looking file whose metadata is known before its bytes are local.

```text
provider process
  publishes metadata
  supplies bytes on read

fuse-promise
  owns the Promise tree
  exposes FUSE paths
  routes lazy reads
  materializes real files

applications
  use normal filesystem APIs
```

Default mount:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

Typical install shape:

```text
/usr/include/fuse-promise/fuse-promise.h
/usr/lib/libfusepromise.so
/usr/lib/pkgconfig/fuse-promise.pc
/usr/lib/systemd/user/fuse-promised.service
```

## Public Interface

The implementation is expected to be written primarily in Rust.

The public interface is not Rust. The stable system interface is a C ABI:

```c
#include <fuse-promise/fuse-promise.h>
```

Public consumers link `libfusepromise.so`. Internal daemon communication is private and may change.

This keeps the component usable from C, C++, Rust, Go, Python, Qt, GTK, desktop services, command-line tools, and other Linux software.

## Boundaries

`fuse-promise` provides:

- Promise filesystem semantics.
- A user-session daemon.
- A FUSE-backed runtime.
- A stable C ABI.
- Provider registration and lazy read routing.
- Materialization into real local files.
- Runtime lifecycle, inode, metadata, and cache policy.

`fuse-promise` does not provide:

- Clipboard synchronization.
- Desktop drag-and-drop adapters.
- Cloud-provider integrations.
- P2P transport.
- Application-specific remote file protocols.

Those projects should live outside this repository and use the public API.

## Documentation

- [Project Statement](docs/project-statement.md)
- [Requirements](docs/requirements.md)
- [Architecture](docs/architecture.md)
- [Implementation Decisions](docs/implementation-decisions.md)
- [Language and ABI](docs/language-and-abi.md)
- [Promise Model](docs/promise-model.md)
- [Public API](docs/public-api.md)
- [Runtime](docs/runtime.md)
- [Development Style](docs/development-style.md)
- [Roadmap](docs/roadmap.md)
- [Progress Goals](docs/progress.md)

## Source Layout

```text
include/fuse-promise/fuse-promise.h  public C ABI
crates/fuse-promise-runtime/         core Promise metadata model
crates/fuse-promise-ipc/             private daemon IPC helpers
crates/fuse-promise-ffi/             libfusepromise C ABI implementation
crates/fuse-promise-daemon/          fuse-promised daemon entry point
tools/fpctl/                         administrative CLI
pkgconfig/fuse-promise.pc.in         pkg-config template
systemd/user/fuse-promised.service   placeholder user service template
```

## Repository Description

Suggested short description for GitHub:

```text
Linux user-space Promise filesystem runtime built on FUSE, with lazy reads and materialize support.
```
