# Project Statement

## About

`fuse-promise` is a Linux user-space filesystem component for promised files.

It allows a provider process to publish a filesystem tree before the file contents are present locally. The tree is visible through ordinary filesystem paths. Metadata is available immediately. File contents are supplied later, on demand, when a process reads the file. A promised file or directory can also be materialized into real local storage through a standard operation.

The project is built on the existing Linux FUSE interface. The kernel FUSE driver remains the kernel boundary. `fuse-promise` implements the filesystem daemon, runtime, public library, provider model, and materialization semantics in user space.

## Position

`fuse-promise` is a system component, not an end-user application.

It belongs in the same broad layer as other Linux user-space filesystem and desktop/runtime infrastructure: small public interfaces, a long-running runtime process when needed, conservative behavior, and clear boundaries between public ABI and private implementation.

The repository provides:

- A Promise filesystem model.
- A user-session daemon.
- A FUSE-backed runtime.
- A stable C ABI.
- A public header and shared library.
- A materialize operation.
- Administrative tools and tests.

The repository does not provide:

- Clipboard synchronization.
- Cloud storage integrations.
- P2P transport.
- Desktop drag-and-drop adapters.
- Application-specific remote file protocols.

Those features are valid users of `fuse-promise`, but they are not part of this repository.

## Rationale

Linux already has a strong kernel/userspace separation for filesystems through FUSE. The kernel provides the VFS integration and `/dev/fuse` communication path. A userspace daemon provides filesystem data and metadata. This is the correct layer for a Promise filesystem because the behavior is policy-heavy, provider-specific, and session-scoped.

Promised files should not require kernel patches. They should not require every application to implement a new protocol. They should appear as normal filesystem paths while exposing a small system API for producers that can supply content lazily.

## Design Goals

- Use the existing Linux FUSE model.
- Run by default as the current user.
- Mount by default under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Expose ordinary filesystem paths to consumers.
- Expose a stable C ABI to providers.
- Keep daemon IPC private and replaceable.
- Make metadata publication cheap.
- Fetch bytes only when read.
- Support offset-based reads.
- Support recursive materialization.
- Keep the first stable model read-only.
- Keep integration-specific code out of the core tree.

## Non-Goals

- No Linux kernel changes.
- No kernel module.
- No clipboard product in this repository.
- No desktop-environment plugin in this repository.
- No cloud-provider implementation in this repository.
- No public daemon wire protocol.
- No language-specific SDK as the primary ABI.

Language bindings may exist later, but they must bind to the C ABI.

## Public Interface Policy

The public interface is:

```text
fuse-promise/fuse-promise.h
libfusepromise.so
fuse-promise.pc
```

The public C ABI must be versioned and conservative. Public structs should be extensible. Public symbols should use the `fp_` prefix. Public handles should be opaque.

The private interface is everything between `libfusepromise.so` and `fuse-promised`. It may use Unix domain sockets, D-Bus, shared memory, or another local transport. Applications must not depend on that transport.

## Language and ABI Policy

The preferred implementation language is Rust.

Rust is an implementation detail used for the runtime, daemon, metadata model, provider session management, cache, materialize engine, IPC implementation, and FUSE adapter code.

Rust is not the public ABI. Public consumers must not depend on Rust crates, Rust traits, Rust generics, Rust ownership types, async futures, Tokio handles, or any other Rust-specific interface.

The stable external contract is the C ABI:

```text
include/fuse-promise/fuse-promise.h
libfusepromise.so
fuse-promise.pc
```

This keeps `fuse-promise` usable from C, C++, Go, Python, Qt, GTK, desktop services, command-line tools, and other Linux ecosystem software without binding them to Rust ABI stability.

## Filesystem Semantics

A committed Promise tree should behave like a normal read-only filesystem tree unless documented otherwise.

Expected behavior:

- `stat` returns declared metadata.
- `readdir` returns declared children.
- `open` validates the node and provider state.
- `read` asks the provider for the requested byte range.
- `cp` and similar tools naturally materialize content by reading it.
- Explicit materialization writes a real local copy and records materialized state.

Errors from provider or runtime failures should map to deterministic `errno` values for filesystem callers and to `fp_status_t` values for public API callers.

## Security Model

The default runtime is user-session scoped.

The mount owner is the current user. The daemon runs with the privileges of that user. The default mount must not expose promised content to other users. Any future support for broader visibility must be explicit and must account for FUSE permission behavior.

The runtime must validate:

- Provider ownership.
- Node identity.
- Read ranges.
- Message sizes.
- Target paths for materialization.
- Provider disconnects.

## Implementation Character

The codebase should look and behave like a Linux infrastructure component:

- Small public surface.
- Plain errors.
- Explicit ownership.
- No hidden application policy in the core.
- No required network stack.
- No required desktop stack.
- No dependency on a single provider protocol.
- Source layout that separates public headers, daemon code, runtime code, FUSE adapter code, tools, and tests.

The project should prefer boring interfaces that distributions can package and downstream projects can rely on.

## Status

The project is currently in the design and specification phase.

The first implementation target is a read-only Promise filesystem MVP with:

- Static promised tree commit.
- `getattr`, `readdir`, `open`, and `read`.
- Provider read callbacks.
- A user-session daemon.
- A minimal administrative CLI.
- File and directory materialization.

## References

- Linux kernel FUSE documentation: https://www.kernel.org/doc/html/latest/filesystems/fuse/
- libfuse project: https://github.com/libfuse/libfuse
- libfuse API documentation: https://libfuse.github.io/doxygen/
