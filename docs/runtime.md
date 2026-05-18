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

`XDG_RUNTIME_DIR` must be present, absolute, owned by the current user, an
existing directory, and not accessible by group or other users. The daemon
should create the `fuse-promise` child directory with user-session scoped
permissions and fail explicitly if the runtime directory is unsafe or
unavailable.

## MVP Visible Layout

The read-only MVP exposes one directory per committed Promise tree directly
under the mount root:

```text
$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/
```

The daemon allocates visible promise identifiers such as `promise-1`. Provider
processes and public clients cannot choose those identifiers. The path returned
by `fp_promise_commit()` is the absolute path to the allocated Promise root.
Declared relative paths are exposed below that root:

```text
$XDG_RUNTIME_DIR/fuse-promise/promise-1/docs/readme.txt
```

The daemon owns inode assignment and keeps inode values stable for the lifetime
of the committed in-memory tree. The developer-preview layout should not be
treated as a stable ABI until the first developer ABI release documents it in
the public API and compatibility tests.

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
disconnected and marks their available promises as provider-gone. Daemon-side
provider read routing exists for the in-process IPC state. Real mounted FUSE
read verification is covered by the smoke harness. File and directory subtree
materialize IPC are implemented for fail-on-conflict behavior. Reads for
materialized files can use the local materialized path after provider
disconnect. An opt-in read-through cache can satisfy fully cached ranges after
provider disconnect; overwrite and rename policies, progress, cancellation,
and read coalescing are still under development.

`libfusepromise.so` provider registration uses this private daemon IPC and no
longer creates authoritative provider sessions in a client-local runtime. Its
provider helper thread dispatches private read requests to the registered
public C callback and writes the private read response back over the provider
connection.

The runtime can plan file reads from committed Promise metadata. A read plan
resolves the owning provider, provider node id, normalized relative path,
offset, and capped length, and rejects missing nodes, directories,
provider-gone state, and disconnected providers before provider IPC is used.
The daemon IPC state can then route that read request over the registered
provider connection and match the response by request id.

## Lifecycle

Mount readiness is explicit:

| State | Status Fields | Commit Behavior |
|---|---|---|
| Adapter unavailable | `mount=not-mounted`, `fuse_adapter=not-implemented` | Reject commit as unavailable. |
| Adapter disabled | `mount=not-mounted`, `fuse_adapter=disabled` | Reject commit as unavailable. |
| Mounted but not commit-ready | `mount=mounted`, `fuse_adapter=enabled` | Reject commit as unavailable. |
| Commit-ready | `mount=mounted`, `fuse_adapter=enabled` | Commit may mutate runtime and return the visible Promise path. |

Only the daemon may transition to commit-ready after the user-session mount and
runtime-backed FUSE adapter are ready. Public clients cannot override mount
readiness.

Recommended lifecycle:

1. User session starts.
2. `fuse-promised` starts on demand or through a user service.
3. The daemon mounts `$XDG_RUNTIME_DIR/fuse-promise/`.
4. Applications register providers and commit promises.
5. Filesystem users access promised paths.
6. Reads are routed to providers or materialized paths.
7. On session exit, daemon unmounts and clears session-scoped state.

The current daemon reports mount state through the same private status IPC.
Default builds keep the adapter disabled; enabling the daemon's `fuse-mount`
feature prepares a private user-owned mountpoint, starts the `fuser` background
session, keeps its handle alive for the daemon lifetime, and explicitly
unmounts and joins the background session when the daemon exits normally. The
feature-gated adapter resolves inodes and directories through the daemon
runtime and routes file reads back to registered providers. Private metadata
commit uses this state as a readiness gate: disabled, unmounted, or mount-only
daemons reject commit before mutating runtime state, while a commit-ready daemon
state can return `$XDG_RUNTIME_DIR/fuse-promise/<promise-id>`.
Promise file opens use FUSE direct I/O so provider reads receive the caller's
actual offset-based read ranges instead of kernel page-cache readahead ranges.
If a file has been fully materialized, the runtime plans reads against the
stored local materialized path before requiring a live provider.
If read-through cache mode is enabled, the runtime may return a complete cached
range before requiring a live provider. Cache misses keep the existing provider
read path. After a full provider read, the daemon may synchronously prefetch the
next sequential range and store it in the same read-through cache.

Until a commit-ready FUSE namespace exists, public commit should return
`FP_ERR_UNAVAILABLE`. Public materialize supports files and directory subtrees
with fail-on-conflict behavior; unsupported materialize modes should return
`FP_ERR_UNAVAILABLE` or a documented error.

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

Initial read-only FUSE mapping:

| Runtime Condition | FUSE Operations | `errno` |
|---|---|---:|
| Missing inode, path, or child | `lookup`, `getattr`, `readdir`, `open`, `read` | `ENOENT` |
| Directory opened or read as a file | `open`, `read` | `EISDIR` |
| File read requested for a provider-gone Promise | `open`, `read` | `EIO` |
| Provider route unavailable or disconnected | `read` | `EIO` |
| Invalid read offset or size | `read` | `EINVAL` |
| Runtime lock or internal IO failure | all FUSE callbacks | `EIO` |

Timeouts and cancellation should map to `ETIMEDOUT` and `ECANCELED` when those
states exist in the runtime.

## Observability

The runtime should eventually expose:

- Active promises.
- Provider sessions.
- Mount state.
- Materialize jobs.
- Read error counters.
- Cache policy and usage.

Observability should be available through `fpctl` and structured logs.
`fpctl status` reports daemon, mount state, and the active cache policy
(`cache_policy=no-cache` by default, `cache_policy=read-through` when enabled).
`fpctl list` reports daemon-owned providers, promises, and runtime nodes through
private IPC.
