# Architecture

## Position in the System

`fuse-promise` sits between ordinary Linux applications and the existing Linux FUSE kernel interface.

```text
Applications and system tools
  call normal filesystem APIs or link libfusepromise.so

Public fuse-promise ABI
  fuse-promise/fuse-promise.h
  libfusepromise.so

Private runtime owned by fuse-promised
  provider sessions
  private daemon IPC
  metadata store
  inode map
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

The library must not be the authoritative Promise namespace. It may hold client
handles and pending builder state, but committed Promise trees, provider
session state, inode allocation, and mount state belong to `fuse-promised`.

The transport between the library and daemon is private implementation detail.
Before that transport exists, public functions that would require a visible
FUSE path must return `FP_ERR_UNAVAILABLE` instead of fabricating paths.

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
- Runtime node identifiers.
- Inode mapping.
- Parent-child directory indexes.
- Provider ownership.
- Promise lifecycle state.
- Read request planning.
- Error mapping.
- Lifecycle state.
- Materialization state.

The runtime crate may be used in unit tests and daemon internals, but the
deployed system has one authoritative runtime instance per user-session daemon.
Per-client in-process runtime state is only acceptable for temporary builder
validation and tests.

### Materialize Engine

The materialize engine converts promised nodes into real files.

It must use the same provider read path as normal lazy reads. This guarantees that materialization and ordinary file access share the same correctness rules.

### Cache Layer

Caching is an optimization, not a requirement for the public model.

Possible cache modes:

- No cache. This is the current default and is reported as
  `cache_policy=no-cache` by `fpctl status`.
- Read-through chunk cache. This is opt-in with `fuse-promised
  --cache=read-through`, coalesces provider reads to cache chunks, stores
  complete read ranges in memory, and prefetches the next sequential range
  after full provider reads.
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

## Runtime Ownership Model

`fuse-promised` owns all state that is visible through the mounted filesystem.

```text
provider application
  -> libfusepromise.so
  -> private IPC client

fuse-promised
  -> provider session table
  -> Promise metadata store
  -> inode and directory indexes
  -> FUSE adapter
```

The daemon is the only process that may allocate visible promise identifiers,
runtime node identifiers, and inode numbers. This avoids split-brain behavior
where one client believes a Promise tree was committed but the mounted
filesystem cannot see it.

Client-side library state is limited to:

- Opaque public handles.
- Provider callback pointers and user data.
- Builder state before commit.
- Private IPC connection state.
- Temporary buffers for provider callbacks.

## Provider Session Model

A provider session is live while its public library connection remains alive.
The daemon tracks each provider session by an internal provider id and state:

```text
live -> disconnected
```

Promises owned by a disconnected provider remain visible only according to
runtime policy:

- If fully materialized, reads may use the materialized path.
- If fully cached, reads may use cache policy.
- Otherwise, reads and materialize operations fail deterministically.

Provider callback pointers are public ABI concepts inside the provider process,
but the callback transport between daemon and provider is private. A future
implementation may use a library-managed helper thread, a Unix socket, shared
memory, eventfd, or another daemon-controlled mechanism. Applications must not
depend on that mechanism.

## Metadata and Inode Model

Each committed tree is a snapshot unless declared otherwise by a future
capability. The daemon stores nodes with:

- Runtime node id.
- Stable inode number for FUSE.
- Provider-owned opaque node id.
- Normalized relative path.
- Parent id or parent path.
- Child index for directories.
- Node kind.
- Size, permission bits, and timestamps.
- Lifecycle state such as available, provider-gone, cached, or materialized.

The public `mode` field represents permission bits, not the full Linux
`st_mode` file type. The FUSE adapter combines node kind and permission bits
when answering `getattr`.
The public `mtime_nsec` field is validated as a non-negative Unix epoch
nanosecond timestamp.

The initial public ABI accepts UTF-8 path strings. If the project requires full
Linux byte-string path coverage before ABI freeze, a byte-path API should be
added before the first stable release.

## Commit Flow

Commit is the boundary where builder state becomes visible filesystem state.

```text
fp_promise_commit()
  -> libfusepromise.so validates public handles and builder state
  -> private IPC commit request
  -> fuse-promised validates provider ownership and paths
  -> daemon allocates promise id, node ids, and inodes
  -> daemon updates metadata store
  -> daemon returns visible path under the mounted namespace
```

If the daemon is unavailable, the FUSE mount is not ready, or the IPC transport
has not been implemented, commit must return `FP_ERR_UNAVAILABLE`.

## Read Flow

```text
application read(2)
  -> Linux VFS
  -> FUSE kernel interface
  -> fuse-promised read callback
  -> runtime resolves inode to Promise node
  -> runtime resolves provider session
  -> daemon asks provider process for offset and length
  -> provider writes into runtime-owned buffer
  -> daemon returns bytes to FUSE
```

Read requests are offset-based. A read past end-of-file returns zero bytes. A
short provider response is allowed and follows normal filesystem semantics.

## Materialize Flow

Materialize uses the same provider read path as ordinary filesystem reads:

```text
fp_materialize()
  -> libfusepromise.so
  -> private IPC materialize request
  -> fuse-promised validates target, policy, and provider state
  -> runtime walks the Promise subtree
  -> runtime reads chunks through the provider read path
  -> runtime writes normal files and applies metadata
  -> runtime records materialized state
```

This keeps materialize behavior aligned with lazy read behavior and avoids a
second provider protocol.

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

Minimum private IPC capabilities:

- Connect to or start the user-session daemon.
- Register and unregister provider sessions.
- Commit Promise tree metadata.
- Route provider read requests and responses.
- Query daemon status for `fpctl`.
- Inspect daemon-owned promises and runtime nodes for `fpctl list`.
- Request materialization, report materialize progress, and cancellation.
- Propagate provider disconnects.

IPC messages must validate size, version, provider ownership, path bounds, read
ranges, and target paths before mutating daemon state.

The current implementation includes bounded framed status, provider
register/unregister, Promise metadata commit IPC, and provider read
request/response message helpers over private Unix sockets. Daemon-side
provider read routing exists inside the private IPC state; real mounted FUSE
read verification is covered by the smoke harness, and file plus directory
subtree materialize IPC supports fail-on-conflict, overwrite, and rename
behavior, progress reporting, and cancellation for Phase 2.
Both paths must remain private to `libfusepromise.so` and
`fuse-promised`. Registered providers are scoped to the IPC connection that
registered them; closing that connection marks those providers disconnected in
the daemon runtime.

The public library now registers providers through this private IPC. It may
still keep builder metadata before commit, but it must not create a committed
client-local Promise namespace.

The public library also owns the provider-side callback dispatch loop. Private
read requests received on the provider connection are converted into
`fp_read_request_t` / `fp_read_response_t` calls inside the provider process.
The daemon IPC state routes daemon-originated read requests into that channel
and validates that responses come back over the registered provider connection.

The daemon runtime plans reads before provider IPC. Planning resolves the
committed Promise tree, enforces provider ownership and provider liveness,
rejects non-file nodes, and caps read length at EOF.
The shared daemon IPC state owns provider connection routes and pending read
requests so read plans can be delivered to the correct provider process and
matched to validated responses.

Runtime directory validation rejects missing, relative, non-directory,
foreign-owned, or group/other-accessible `XDG_RUNTIME_DIR` paths before mount
or control socket paths are derived.

The daemon owns mount lifecycle reporting. Default builds keep the FUSE adapter
disabled so the workspace remains buildable without libfuse3 development
packages; builds with the `fuse-mount` feature create the user-session
mountpoint and hold the `fuser` background session handle for daemon lifetime.
Mountpoint preparation creates a private `0700` directory and rejects unsafe
existing paths before the FUSE session starts.
The daemon keeps the background session handle in a mount wrapper that
explicitly unmounts and joins the session on normal daemon shutdown.
The feature-gated adapter implements read-only FUSE callbacks over the runtime
inode and directory views and uses the daemon provider route for offset reads.
Private metadata commit checks shared commit readiness before mutating the
runtime. A commit-ready daemon state returns the future visible promise path;
disabled, unmounted, or mount-only daemon state returns unavailable and leaves
the runtime unchanged.

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
