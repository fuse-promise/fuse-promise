# Project Statement

## Summary

`fuse-promise` is a Linux user-space filesystem component for promised files.

It lets a provider publish a filesystem tree from metadata first, expose that tree through ordinary paths, supply file contents only when the file is read, and materialize promised nodes into real local storage when requested.

## Project Identity

`fuse-promise` is infrastructure.

It is intended to behave like a Linux ecosystem component: small public surface, stable ABI discipline, conservative runtime behavior, clear security boundaries, and implementation details hidden behind installed headers and shared libraries.

It is not an application framework and not a product-specific sync client.

## System Layer

The project sits above the Linux FUSE kernel interface and below application-specific systems.

```text
Applications and providers
  use normal filesystem APIs or libfusepromise.so

fuse-promise
  public C ABI
  user-session daemon
  Promise runtime
  FUSE adapter

Linux
  VFS
  fuse.ko
  /dev/fuse
```

The kernel is not modified. Promise behavior is implemented by the user-space daemon.

## Core Idea

A Promise tree separates file existence from file availability.

The runtime can expose:

- File names.
- Directory structure.
- File sizes.
- Modes.
- Timestamps.
- Provider-owned node identifiers.

without requiring local file content to exist yet.

When an ordinary process reads a promised file, the runtime asks the owning provider for the requested byte range. When a caller needs a real local copy, the runtime materializes the promised node by reading the provider stream and writing normal files.

## Goals

- Provide a generic promised-file layer for Linux.
- Use the existing FUSE model instead of kernel patches.
- Run by default as the current user.
- Mount by default under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Expose ordinary filesystem paths to consumers.
- Expose a versioned C ABI to providers.
- Keep daemon IPC private.
- Support metadata-only tree creation.
- Support offset-based lazy reads.
- Support recursive materialization.
- Keep the first stable filesystem model read-only.

## Non-Goals

- No Linux kernel module.
- No kernel changes.
- No clipboard product in this repository.
- No desktop-environment plugin in this repository.
- No cloud-provider implementation in this repository.
- No P2P transport implementation in this repository.
- No public daemon wire protocol.
- No Rust ABI as the public interface.

Upper-layer software may implement clipboard, cloud, P2P, or desktop behavior by using the public ABI.

## Public Interface

The stable public contract is:

```text
fuse-promise/fuse-promise.h
libfusepromise.so
fuse-promise.pc
```

Public symbols should use the `fp_` prefix. Public handles should be opaque. Public structs should be versionable or size-tagged where future extension is expected.

Everything between `libfusepromise.so` and `fuse-promised` is private implementation detail.

## Language Policy

The preferred implementation language is Rust.

Rust may be used for the daemon, runtime, metadata model, provider session management, materialize engine, cache, private IPC, FUSE adapter, and tools.

Rust must not be the public ABI. Public consumers must not depend on Rust crates, traits, generics, async futures, Tokio handles, or Rust ownership types.

## Filesystem Behavior

A committed Promise tree should behave like a normal read-only filesystem tree unless documented otherwise.

Expected behavior:

- `stat` returns declared metadata.
- `readdir` returns declared children.
- `open` validates the node and provider state.
- `read` requests bytes from the provider by node, offset, and length.
- `cp`, `tar`, `rsync`, and similar tools work by reading files normally.
- Explicit materialization writes real files and records materialized state.

Runtime failures should map to deterministic `errno` values for filesystem callers and to `fp_status_t` values for public API callers.

## Security Position

The default runtime is user-session scoped.

The daemon runs as the current user and owns the session mount. The default mount must not expose promised content to other users. Any broader visibility must be explicit.

The runtime must validate:

- Provider ownership.
- Node identity.
- Paths and path traversal.
- Read ranges.
- Message sizes.
- Materialize target paths.
- Provider disconnects.

## Implementation Character

The codebase should be plain, inspectable, and distribution-friendly.

Prefer:

- Small APIs.
- Typed errors.
- Explicit ownership.
- Stable install paths.
- Boring command-line tools.
- Clear separation between public ABI and private runtime.

Avoid:

- Hidden application policy.
- Required desktop dependencies.
- Required network dependencies.
- Provider-specific logic in core.
- Integration directories inside the core repository.

## Initial Target

The first implementation should prove the minimum system behavior:

- Start a user-session daemon.
- Mount `$XDG_RUNTIME_DIR/fuse-promise/`.
- Commit a static Promise tree.
- Serve `getattr`, `readdir`, `open`, and `read`.
- Route reads to provider callbacks.
- Materialize a file and a directory tree.
- Expose a small `fpctl` utility for inspection and testing.

## References

- Linux kernel FUSE documentation: https://www.kernel.org/doc/html/latest/filesystems/fuse/
- libfuse project: https://github.com/libfuse/libfuse
- libfuse API documentation: https://libfuse.github.io/doxygen/

