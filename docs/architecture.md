# Architecture

## Position in the System

`fuse-promise` sits between ordinary Linux applications and the existing Linux FUSE kernel interface.

```text
Applications and system tools
  call normal filesystem APIs or link libfusepromise.so

Public fuse-promise ABI
  fuse-promise/fuse-promise.h
  libfusepromise.so

Private runtime
  provider sessions
  daemon IPC
  metadata store
  cache
  materialize engine

FUSE filesystem daemon
  getattr
  readdir
  open
  read
  release

Linux kernel
  VFS
  fuse.ko
  /dev/fuse
```

The kernel is not modified. The daemon implements filesystem behavior in user space.

## Implementation Language Boundary

The preferred implementation language is Rust, but Rust must remain behind the system boundary.

```text
External applications
  -> C ABI
  -> libfusepromise.so
  -> Rust implementation internals
  -> fuse-promised
  -> FUSE
```

Rust may be used for:

- Runtime state management.
- Provider session tracking.
- Metadata and inode mapping.
- Cache implementation.
- Materialize engine.
- Private IPC.
- FUSE adapter implementation.
- CLI implementation.

Rust must not leak into the public ABI. Public headers must not expose Rust types, Rust ownership rules, Rust traits, async futures, runtime handles, or crate-level APIs.

## Major Components

### Public Library

`libfusepromise.so` is the only supported programmatic entry point.

Responsibilities:

- Provide a stable C ABI.
- Create and manage client contexts.
- Register provider sessions.
- Build and commit promised trees.
- Call materialize operations.
- Hide private daemon communication.
- Translate runtime errors into public error codes.

The transport between the library and daemon is private implementation detail.

### User-Session Daemon

`fuse-promised` owns the mounted Promise filesystem for the current user session.

Responsibilities:

- Start and maintain the FUSE mount.
- Own the metadata index.
- Own inode allocation.
- Route read requests to provider sessions.
- Track provider ownership.
- Enforce lifecycle and cleanup policy.
- Coordinate materialization.
- Maintain optional cache state.

### FUSE Adapter

The FUSE adapter maps Linux filesystem operations to Promise runtime operations.

Typical mapping:

```text
getattr -> declared node metadata
readdir -> declared child names
open    -> validate readable node and provider availability
read    -> provider read callback or local materialized path
release -> close runtime file handle
```

The FUSE adapter should be thin. Promise semantics belong in the core runtime, not inside callback glue.

### Core Runtime

The core runtime owns provider-independent behavior:

- Promise tree model.
- Node metadata validation.
- Path normalization.
- Inode mapping.
- Provider ownership.
- Read request planning.
- Error mapping.
- Lifecycle state.
- Materialization state.

### Materialize Engine

The materialize engine converts promised nodes into real files.

It must use the same provider read path as normal lazy reads. This guarantees that materialization and ordinary file access share the same correctness rules.

### Cache Layer

Caching is an optimization, not a requirement for the public model.

Possible cache modes:

- No cache.
- Read-through chunk cache.
- Materialized-file passthrough.
- Provider-defined cache policy.

Cache policy must not change visible Promise semantics.

## Repository Boundary

The repository should not contain application-specific integrations.

Allowed:

- Core runtime.
- Public C header.
- Shared library implementation.
- Daemon.
- CLI for administrative and test operations.
- Tests and minimal samples that exercise the public API.

Not allowed in the core tree:

- Clipboard synchronization products.
- Wayland or X11 clipboard adapters.
- Cloud provider integrations.
- P2P transport implementation.
- Desktop-environment plugins.

External projects should use the public API.

## Provider Model

A provider is a process that owns promised content and can satisfy read requests.

The provider uses `libfusepromise.so` to:

1. Open a runtime context.
2. Register provider callbacks.
3. Create a promised tree.
4. Commit the tree into the FUSE namespace.
5. Stay alive while reads may occur.
6. Optionally call materialize or destroy.

This mirrors the general platform pattern used by file promise systems: the producer declares file metadata first and supplies data later when the consumer actually requests it.

## Mount Model

Default mount path:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

The path is user-session scoped and should be removed on session exit.

The runtime may expose committed promises under stable subdirectories:

```text
$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/
```

The exact visible layout is part of the filesystem compatibility contract and must be documented before the first stable release.

## Private IPC

The daemon and public library require a communication channel, but it is not public API.

Acceptable implementations include:

- Unix domain sockets.
- D-Bus.
- Shared memory plus eventfd.
- A future custom transport.

Applications must not depend on this channel directly. They must link the shared library.

## Failure Model

The runtime must produce deterministic failures for:

- Missing provider.
- Provider timeout.
- Invalid read range.
- Permission failure.
- Provider cancellation.
- Materialize target conflict.
- FUSE mount failure.
- Runtime version mismatch.

Errors should be surfaced through public `fp_status_t` values and mapped to suitable `errno` values for filesystem callers.
