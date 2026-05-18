# Implementation Decisions

This document fixes the implementation plan before expanding code. It is not a
public ABI contract. It records internal dependency choices, rejected
alternatives, and the order in which the core system should be built.

`fuse-promise` remains a Linux user-space Promise filesystem component. The
core repository must not grow clipboard, cloud, P2P, desktop, or
application-specific integrations.

## Decision Summary

- Implementation language: Rust.
- Public ABI: stable C ABI in `include/fuse-promise/fuse-promise.h`.
- Minimum Rust version: 1.85.
- FUSE backend: `fuser` with libfuse3 mount support.
- FUSE system dependency: Linux FUSE kernel interface plus `fusermount3` /
  libfuse3 runtime tooling.
- Daemon model: `fuse-promised` owns the authoritative runtime.
- IPC model: private Unix domain socket under `XDG_RUNTIME_DIR`.
- IPC encoding: length-prefixed `bincode` messages.
- Runtime model: daemon-owned metadata, provider sessions, node ids, inodes,
  materialize state, and cache state.
- Async model: no Tokio or async runtime in the first read-only MVP.
- CLI parser: manual parsing is acceptable for the current tiny command set;
  `clap` is the approved parser once commands or options expand.
- Logging: `tracing` and `tracing-subscriber`.
- Internal errors: `thiserror`.
- Tests: `tempfile` for isolated runtime directories and filesystem tests.
- Packaging: keep the public header hand-maintained for now; generate install
  metadata later from existing templates.

## Architecture Lock

The first implementation must preserve this dependency direction:

```text
external applications
  -> public C ABI
  -> libfusepromise.so
  -> private IPC
  -> fuse-promised
  -> runtime model
  -> FUSE adapter
  -> Linux FUSE kernel interface
```

The daemon is the first process that may own visible Promise state. The public
library may validate handles and build pending metadata, but committed trees,
visible promise ids, runtime node ids, inode numbers, provider session state,
mount state, and materialize state belong to `fuse-promised`.

Crate ownership:

| Crate | Owns | Must Avoid |
|---|---|---|
| `fuse-promise-runtime` | Provider-independent metadata, node validation, path normalization, inode/node maps, lifecycle state, read planning, materialize planning. | FUSE crate types, CLI parsing, public C ABI, private socket implementation. |
| `fuse-promise-ipc` | Private message framing, Unix socket helpers, daemon status, provider/commit/read/materialize message families. | Public ABI exposure, policy decisions that belong to the runtime. |
| `fuse-promise-ffi` | Public C ABI, opaque handles, panic boundaries, public status mapping, provider callback dispatch in the provider process. | Authoritative committed namespace, direct FUSE operations, public exposure of IPC structs. |
| `fuse-promise-daemon` | User-session daemon lifecycle, authoritative runtime instance, FUSE mount lifecycle, IPC server, provider ownership enforcement. | Application-specific integrations, public ABI definitions. |
| `fpctl` | Administrative and diagnostic commands through private daemon APIs. | Becoming the primary application API. |

Dependencies should be added to individual crates only when the implementation
uses them. The workspace manifest fixes allowed versions; it is not a reason
to import every dependency everywhere.

The daemon exposes FUSE mounting behind its `fuse-mount` feature. This keeps
the default workspace build green on machines that have only the FUSE runtime
tools installed. Enabling the feature requires `pkg-config` and libfuse3
development metadata so `fuser` can link the libfuse3 mount implementation.

## Dependency Table

| Area | Decision | Version | First Used | Notes |
|---|---|---:|---|---|
| FUSE adapter | `fuser` | `0.17.0` | Phase 1 | Use `default-features = false`, `libfuse3`, and `abi-7-31`. This targets libfuse3/fusermount3 while keeping Promise semantics in our runtime. |
| IPC encoding | `bincode` | `2.0.1` | Phase 1 | Use only for private IPC. Public consumers must never depend on the wire format. |
| Unix/POSIX checks | `rustix` | `1.1` | Phase 1 | Use for peer credentials, runtime directory validation, timestamp application, and safer Unix filesystem/process operations. |
| CLI | `clap` | `4.5` | Phase 1 | Approved for `fpctl` and `fuse-promised` once the CLI expands beyond the current tiny command set. |
| Logging | `tracing` | `0.1` | Phase 1 | Structured daemon and CLI events. |
| Logging subscriber | `tracing-subscriber` | `0.3` | Phase 1 | Foreground/debug logging and future systemd-friendly formatting. |
| Internal errors | `thiserror` | `2.0` | Phase 1 | Runtime/IPC/daemon internal errors; public ABI still returns `fp_status_t`. |
| Temp paths/tests | `tempfile` | `3.27` | Phase 1 | Integration and IPC tests with isolated `XDG_RUNTIME_DIR`. |

These versions are declared in the workspace manifest so implementation work
uses one dependency set.

## Dependency Freeze

The Phase 1 dependency set is frozen. New runtime, IPC, daemon, FFI, or CLI
dependencies must not be added casually while implementing read-only behavior.
If a new dependency becomes necessary, update this document first with the
reason, owning crate, feature gate, rejected alternatives, and verification
impact.

Reproducibility is enforced with `Cargo.lock` and `--locked` verification
commands. The manifest keeps compatible version requirements instead of exact
`=x.y.z` pins unless a dependency-specific reason requires exact pinning. Do
not run `cargo update` as part of ordinary feature work.

Rules:

- Keep the default workspace build independent of system FUSE development
  packages. `cargo check --workspace --locked` and `cargo test --workspace
  --locked` must work without `pkg-config` or libfuse3 headers.
- Keep `fuser` isolated to `fuse-promise-daemon` behind the `fuse-mount`
  feature. Runtime, IPC, FFI, and `fpctl` must not import FUSE crate types.
- Use `bincode` only for private IPC. It is not an ABI, file format, or public
  compatibility promise.
- Use `rustix` only for Unix/POSIX checks where the standard library does not
  provide the required ownership, permission, credential, timestamp, or socket
  behavior.
- Keep `tempfile` as a dev-dependency for isolated tests.
- `clap`, `tracing`, `tracing-subscriber`, and `thiserror` are approved
  dependencies, but individual crates should add them only when the code starts
  using structured CLI parsing, logging, or internal error enums.
- Do not add Tokio, another async runtime, D-Bus, HTTP clients, database
  engines, cloud SDKs, cache libraries, `cbindgen`, or desktop integration
  dependencies during the read-only MVP.

System dependency matrix:

| Scope | Required Outside Cargo | Notes |
|---|---|---|
| Default build and tests | Rust 1.85+ toolchain | No system FUSE development package should be required. |
| FUSE feature build | `pkg-config`, libfuse3 development metadata | Required by `fuser` with the `libfuse3` feature. |
| FUSE runtime smoke | Linux FUSE kernel support, `/dev/fuse`, `fusermount3`, `fuse3` runtime tools | Needed to verify a real user-session mount. |
| Installed developer ABI | C compiler, C++ compiler, `pkg-config` | Used for header parsing, examples, and generated `fuse-promise.pc`. |

The project targets the libfuse3/fusermount3 stack. Legacy libfuse2 is not a
first implementation target.

## Implementation Order Lock

Build the code in framework-sized loops, each ending with one verification pass
and one pushable commit.

1. Finalize dependency and architecture documents.
2. Replace status-only IPC with bounded framed IPC.
3. Move daemon runtime ownership behind IPC-visible state.
4. Add provider registration and lifecycle routing.
5. Add metadata commit IPC and daemon-side snapshot validation.
6. Add user-session FUSE mount lifecycle.
7. Add read-only FUSE operations over committed metadata.
8. Route offset reads from FUSE back to provider callbacks.
9. Add materialize using the same read path.
10. Harden ABI, install metadata, cache, and performance work.

Do not start materialize or cache before the read-only FUSE path is complete
end to end.

Before adding code to any goal, identify the owning crate, keep the write scope
inside that ownership boundary, and define the observable exit check. A change
that crosses public ABI, private IPC, daemon runtime, and FUSE behavior should
be split into pushable milestones unless the split would leave the system in an
unbuildable state.

## FUSE Backend Decision

Use `fuser` as the Rust FUSE adapter crate.

Configuration:

```toml
fuser = { version = "0.17.0", default-features = false, features = ["abi-7-31", "libfuse3"] }
```

Daemon feature gate:

```toml
fuse-mount = ["dep:fuser"]
```

System packages expected for development and runtime:

```text
fuse3
libfuse3-dev
pkg-config
```

The project is therefore targeting the modern libfuse3/fusermount3 stack, not
the legacy libfuse2 stack.

The FUSE adapter must stay thin:

- Resolve inodes into runtime nodes.
- Map `lookup`, `getattr`, `readdir`, `open`, `read`, and `release`.
- Open Promise files with direct I/O so lazy provider reads preserve caller
  offset and size rather than kernel page-cache readahead ranges.
- Convert runtime errors into `errno`.
- Avoid embedding Promise policy inside FUSE callback glue.

Promise semantics remain in the runtime and daemon.

### Rejected FUSE Alternatives

| Alternative | Reason |
|---|---|
| `fuse3` crate latest | `fuse3 0.9.0` requires Rust 1.91, which is too high for the project baseline. It also pushes the implementation toward async runtime choices before the read-only MVP needs them. |
| Legacy libfuse2 default | The target Linux stack should be libfuse3/fusermount3. libfuse2 compatibility is not a first implementation goal. |
| Direct low-level `/dev/fuse` protocol implementation | Too much protocol surface for the MVP. We can revisit only if FUSE crate behavior blocks required semantics. |
| Kernel module | Out of scope. `fuse-promise` is user-space only. |

## MSRV Decision

The workspace `rust-version` is `1.85`.

Reasons:

- `fuser 0.17.0` requires Rust 1.85.
- `bincode 2.0.1` also requires Rust 1.85.
- Rust 1.85 is a reasonable floor for current development while avoiding the
  much higher Rust 1.91 requirement from the latest `fuse3` crate.

Do not raise MSRV again without a documented dependency reason.

## Private IPC Decision

The private daemon IPC is a Unix domain socket under:

```text
$XDG_RUNTIME_DIR/fuse-promise.sock
```

The original status-only line protocol has been replaced with a bounded framed
protocol:

```text
u32 little-endian length
bincode-encoded private message body
```

Minimum message families:

- `Hello` / version negotiation. Implemented for status.
- `Status`. Implemented.
- `ProviderRegister`. Implemented as a private message.
- `ProviderUnregister`. Implemented as a private message.
- `PromiseCommit`. Implemented as private daemon-owned metadata commit.
- `ProviderReadRequest`. Implemented as bounded private message helpers.
- `ProviderReadResponse`. Implemented as bounded private message helpers.
- Provider disconnect propagation. Implemented for connection-scoped provider
  registrations.
- `Materialize`. Implemented for files and directory subtrees with
  fail-on-conflict behavior.
- `MaterializeCancel`.
- `MaterializeStatus`.
- Structured error response.

IPC validation rules:

- Reject unknown protocol versions.
- Reject frames larger than the configured maximum.
- Validate Unix peer credentials where available.
- Validate provider ownership before daemon state mutation.
- Validate normalized paths, node ids, read ranges, and target paths.
- Keep all IPC message types in internal crates.

Applications must not talk to this socket directly. Public consumers use only
the C ABI.

## Daemon and Concurrency Decision

For the read-only MVP, use a blocking daemon model:

- One authoritative `Runtime` owned by `fuse-promised`.
- Synchronous `fuser` callbacks.
- Standard threads for IPC clients and provider callback bridging.
- No Tokio dependency in Phase 1.

This keeps the initial daemon inspectable and reduces dependency pressure. Add
async only if provider read routing or materialize concurrency proves that the
blocking model is insufficient.

## Provider Callback Bridge

Provider callbacks are public ABI concepts inside the provider process. The
daemon cannot call C callbacks directly across process boundaries.

The planned bridge is:

```text
daemon read request
  -> private IPC to provider process
  -> libfusepromise.so dispatches public C callback
  -> provider writes runtime-owned buffer
  -> libfusepromise.so returns read response over private IPC
  -> daemon returns bytes to FUSE
```

The library may use a helper thread to receive daemon read requests while the
provider process remains alive.

## Public ABI Decision

Keep the C header hand-maintained for now. Do not introduce `cbindgen` until
there is clear benefit.

Rules:

- Public symbols use `fp_`.
- Public handles are opaque.
- Extensible public structs include `struct_size`.
- Public status and policy values use fixed-width integer typedefs.
- Rust types, async types, IPC messages, and daemon internals never appear in
  the public header.
- Every public FFI function catches panics and returns `fp_status_t`.

## CLI Decision

The current `fpctl status`, `fpctl list`, and `fuse-promised --foreground`
command surfaces may keep simple manual parsing. Introduce `clap` before adding
option-rich commands or any command whose parsing rules are no longer trivial.

Initial commands:

- `fpctl status`. Implemented.
- `fpctl list`. Implemented.
- `fpctl inspect <promise-path>`. Planned.
- `fpctl materialize <promise-path> <target-dir>`. Implemented for files and
  directory subtrees with fail-on-conflict behavior.
- `fpctl destroy <promise-path>`. Planned.

`fpctl` remains administrative and diagnostic. It is not the primary
application API.

## Logging and Error Decision

Use:

- `tracing` for daemon, runtime, IPC, FUSE, and materialize events.
- `tracing-subscriber` for foreground logging and later systemd-friendly
  output.
- `thiserror` for internal Rust error enums.

Public errors remain `fp_status_t`; filesystem callers receive `errno`.

## Materialize Decision

Use the same provider read path for lazy reads and materialize. Do not add a
second provider protocol for materialize.

Materialize implementation order:

1. Single file. Implemented.
2. Recursive directory tree. Implemented.
3. Conflict policies.
4. Progress.
5. Cancellation.
6. Materialized-file read passthrough.

Use `std::fs` initially. Add specialized copy or async IO dependencies only if
profiling shows a real need.

## Cache Decision

The first read-only MVP is no-cache.

Later cache work must preserve visible Promise semantics:

- Random reads must not force full-file download.
- Partial cache is not enough to survive provider disconnect unless the
  requested range is fully cached.
- Complete materialized content may satisfy future reads.
- Cache policy must be observable through `fpctl`.

Do not add cache dependencies until the no-cache read and materialize paths are
correct.

## Packaging Decision

Keep placeholders for now:

- `pkgconfig/fuse-promise.pc.in`
- `systemd/user/fuse-promised.service`

Packaging dependencies and tools are deferred until ABI hardening:

- Generate `fuse-promise.pc`.
- Install public header.
- Install `libfusepromise.so`.
- Install `fuse-promised`.
- Install `fpctl`.
- Install user service.
- Define soname/version policy.

## Implementation Order

Do not start with FUSE callbacks. Build the lower layers in this order:

1. Replace status-only IPC with the framed protocol. Done for status.
2. Move provider registration through daemon-owned runtime.
3. Move Promise commit through daemon-owned runtime.
4. Add a test provider path that can answer read requests over IPC.
5. Add FUSE mount and metadata-only `lookup/getattr/readdir`.
6. Add `open/read/release`.
7. Add CLI inspection commands.
8. Add materialize.
9. Harden ABI and packaging.
10. Add cache/performance work.

This order avoids building FUSE logic on top of client-local state.

## Verification Gates

Before each implementation milestone:

```sh
cargo fmt --check --all
cargo check --workspace --locked
cargo test --workspace --locked
git diff --check
```

Before FUSE merge points:

```sh
pkg-config --exists fuse3
test -e /dev/fuse
which fusermount3
```

Before ABI merge points:

```sh
cc -Iinclude -Wall -Wextra -fsyntax-only sample.c
c++ -Iinclude -Wall -Wextra -fsyntax-only sample.cc
nm -D --defined-only target/debug/libfusepromise.so
```
