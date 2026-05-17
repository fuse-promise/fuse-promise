# fuse-promise Requirements

## Purpose

`fuse-promise` provides a generic Linux foundation for promised files.

A promised file is a filesystem node whose metadata is known before its content exists locally. The node behaves like a normal local file to applications, but its bytes are supplied lazily by a provider when the file is opened and read. A promised directory behaves like a normal directory whose children are declared by metadata first.

The project exists to make this capability available as a reusable system component rather than as a feature embedded inside one clipboard, sync, or cloud application.

## Product Statement

`fuse-promise` is a user-space FUSE runtime and public C ABI for creating, exposing, reading, and materializing promised filesystem trees on Linux.

It is inspired by the platform-level semantics of macOS file promises and Windows virtual file/provider models, but it is designed for Linux using the existing FUSE kernel interface and a stable user-space library boundary.

## Primary Goals

- Provide a generic Promise filesystem capability for Linux.
- Run entirely in user space on top of the existing FUSE kernel driver.
- Expose promised files through normal filesystem paths.
- Make upper-layer applications interact through a stable header and shared library.
- Keep all internal runtime communication private and replaceable.
- Support lazy content supply through provider callbacks.
- Support random reads by offset and size.
- Support recursive materialization from promised nodes into real local files.
- Support session-scoped mount lifecycle under `$XDG_RUNTIME_DIR`.
- Avoid binding the core repository to clipboard, desktop, cloud, or transport-specific integrations.

## Non-Goals

- Do not modify the Linux kernel.
- Do not ship a kernel module.
- Do not define a clipboard product inside this repository.
- Do not include desktop-environment-specific integration layers in the core tree.
- Do not expose daemon IPC as a public API.
- Do not require root for the default user-session runtime.
- Do not require applications to know whether Unix sockets, D-Bus, shared memory, or another private transport is used internally.

## Users

The direct users of `fuse-promise` are application and system software developers who need to expose delayed files through normal Linux paths.

Examples of possible external users:

- Remote clipboard tools.
- File transfer tools.
- Cloud sync clients.
- Remote workspace clients.
- Virtual asset loaders.
- Cross-device file handoff systems.
- Desktop file managers or portals.

These users should depend on `libfusepromise.so` and the public header, not on private daemon internals.

## Functional Requirements

### Promise Tree Creation

The public API must allow a provider process to create a promised tree containing files and directories.

For each promised node, the provider must be able to declare:

- Stable node identifier.
- Relative path inside the promised tree.
- Node type.
- File size when known.
- Mode and permissions.
- Modification time.
- Optional provider-owned opaque metadata.

The runtime must commit the tree into the FUSE mount and return a local filesystem path.

### Lazy Read

When a process reads a promised file, the FUSE runtime must request bytes from the owning provider.

The read contract must include:

- Promise identifier.
- Relative path or node identifier.
- Offset.
- Requested byte count.
- Buffer or stream response.

The provider must be able to return partial data, end-of-file, or a structured error.

### Directory Enumeration

The runtime must expose declared directories through standard directory operations.

Directory enumeration should not require file content transfer.

### Materialize

The public API must provide a materialize operation that copies promised content into real local filesystem paths.

Materialize must support:

- Single file.
- Directory subtree.
- Recursive tree copy.
- Existing target conflict policy.
- Progress reporting.
- Cancellation.

After materialization, the runtime may mark nodes as materialized and may map future reads to the real local path if policy allows it.

### Provider Lifetime

The provider process must remain available while its promises can be read, unless the promise was materialized or the runtime has a complete cache.

If the provider disappears, the runtime must expose deterministic errors on affected reads and materialize operations.

### Runtime Lifecycle

The default runtime is user-session scoped.

The default mount path is:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

If `XDG_RUNTIME_DIR` is unavailable, the runtime may fail explicitly instead of falling back to an unsafe shared path.

### API Stability

The public API must be a C ABI.

The project must install:

```text
fuse-promise/fuse-promise.h
libfusepromise.so
fuse-promise.pc
```

The C ABI must be versioned. Internal daemon protocols are not part of the ABI.

## Non-Functional Requirements

### Security

- The default runtime must run as the current user.
- Promises must be isolated by user session.
- Providers must not be able to mutate promises they do not own.
- Provider callbacks must be authenticated by runtime-owned handles or session credentials.
- Private runtime IPC must validate message size, node ownership, and request bounds.

### Performance

- Creating a large promised tree should be metadata-only.
- Reading a file should request only the byte ranges required by the kernel or application.
- Sequential reads should allow prefetching as an implementation detail.
- Random reads must not force full-file download.
- Materialize should stream data without requiring the whole file in memory.

### Compatibility

Promised files should work with ordinary Linux programs that use standard filesystem calls such as:

- `open`
- `stat`
- `readdir`
- `read`
- `pread`
- `cp`
- `tar`
- `rsync`

Some tools may force full reads by design. That behavior is acceptable because it follows normal filesystem semantics.

## Success Criteria

The first complete release should demonstrate:

- A provider process can create a promised directory tree.
- The tree appears under the user-session FUSE mount.
- `ls`, `stat`, and directory traversal work without transferring file content.
- `cat` or `cp` triggers lazy reads.
- `fpctl materialize` or the C API materialize call writes real local files.
- The same core runtime can be used by any external application without repository-level integration code.

