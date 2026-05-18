# Runtime

## Process Model

The default runtime consists of:

```text
application process
  links libfusepromise.so

fuse-promised
  user-session daemon
  owns FUSE mount and Promise metadata

Linux FUSE
  kernel interface
```

The public library may start or connect to the daemon according to
implementation policy. The daemon owns the authoritative runtime state; client
processes must not commit Promise trees into a private in-process namespace.

## Daemon Name

The recommended daemon name is:

```text
fuse-promised
```

## CLI Name

The recommended administrative CLI name is:

```text
fpctl
```

The CLI is for inspection, testing, and materialization. It is not the primary application API.

## Mount Path

Default:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

Typical resolved path:

```text
/run/user/1000/fuse-promise/
```

The runtime should avoid a global shared default mount because Promise ownership and provider callbacks are user-session scoped.

`XDG_RUNTIME_DIR` must be present and absolute. The daemon should create the
`fuse-promise` child directory with user-session scoped permissions and fail
explicitly if the runtime directory is unsafe or unavailable.

## Daemon Responsibilities

- Mount and unmount the FUSE filesystem.
- Keep the Promise metadata index.
- Allocate runtime inode ids.
- Allocate visible promise ids and runtime node ids.
- Maintain parent-child indexes for directory enumeration.
- Route read requests to the correct provider.
- Detect provider disconnects.
- Enforce access and ownership rules.
- Execute materialize jobs.
- Clean up expired promises.
- Report runtime state to `fpctl`.

## Provider Session

A provider session is created when an application registers provider callbacks through the public library.

The provider session is live only while the application process and library connection remain available.

The daemon must invalidate or mark unavailable all non-materialized promises owned by a disconnected provider, unless a configured cache mode can satisfy reads without the provider.

Provider state should be explicit:

```text
live
disconnected
```

Promise state should also be explicit:

```text
available
provider-gone
cached
materialized
destroyed
```

The first implementation may omit cache and destroyed states internally, but
the state machine should leave room for them.

## Internal Communication

The internal communication channel is intentionally private.

Implementation may use:

- Unix domain sockets.
- D-Bus.
- Shared memory.
- Another local IPC mechanism.

The public contract is the C ABI. Applications must not talk to the daemon protocol directly.

Minimum IPC operations:

- Runtime status.
- Provider register and unregister.
- Promise commit.
- Provider read request and response.
- Materialize start, progress, cancellation, and result.

The current implementation provides runtime status, provider
register/unregister messages, Promise metadata commit, and provider read
request/response message helpers over a bounded framed protocol on private Unix
sockets. A provider connection closing marks its registered providers as
disconnected and marks their available promises as provider-gone. Read routing
and other IPC operations are still under development.

`libfusepromise.so` provider registration uses this private daemon IPC and no
longer creates authoritative provider sessions in a client-local runtime.

## Lifecycle

Recommended lifecycle:

1. User session starts.
2. `fuse-promised` starts on demand or through a user service.
3. The daemon mounts `$XDG_RUNTIME_DIR/fuse-promise/`.
4. Applications register providers and commit promises.
5. Filesystem users access promised paths.
6. Reads are routed to providers or materialized paths.
7. On session exit, daemon unmounts and clears session-scoped state.

Until daemon IPC and the FUSE mount exist, public commit and materialize calls
should return `FP_ERR_UNAVAILABLE`.

## Materialize Runtime Flow

```text
fp_materialize()
  -> libfusepromise.so
  -> private runtime request
  -> fuse-promised validates target and promise ownership
  -> runtime walks Promise tree
  -> runtime reads provider content in chunks
  -> runtime writes real local files
  -> runtime applies metadata
  -> runtime records materialized state
```

## Error Mapping

Filesystem errors should map to standard `errno` values where possible.

Public library errors should map to `fp_status_t`.

Examples:

```text
provider unavailable -> EIO or ENXIO
node not found       -> ENOENT
permission denied    -> EACCES
timeout              -> ETIMEDOUT
cancelled            -> ECANCELED
```

## Observability

The runtime should eventually expose:

- Active promises.
- Provider sessions.
- Mount state.
- Materialize jobs.
- Read error counters.
- Cache usage.

Observability should be available through `fpctl` and structured logs.
