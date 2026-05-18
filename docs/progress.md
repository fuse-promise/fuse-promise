# Progress Goals

This document is the working goal checklist for `fuse-promise`.

It tracks the project as a Linux user-space Promise filesystem component. It
does not include clipboard products, cloud providers, P2P transports, desktop
plugins, or application-specific integrations.

## Status Legend

- `[x]` Done in the current repository.
- `[~]` Started, but not yet complete.
- `[ ]` Not started.

## Current Baseline

- [x] Repository scope is documented as a generic Linux user-space FUSE system
  component.
- [x] Public boundary is documented as `fuse-promise/fuse-promise.h`,
  `libfusepromise.so`, and `pkg-config --cflags --libs fuse-promise`.
- [x] Rust workspace exists with runtime, private IPC, public FFI, daemon, and
  CLI crates.
- [x] Public C header exists with opaque handles, fixed-width status values,
  versioned structs, provider read callback types, builder functions, and
  materialize options.
- [x] Runtime crate has an in-memory metadata model with provider sessions,
  Promise trees, node attributes, path normalization, parent/child indexes,
  provider disconnect state, and mode/directory size validation.
- [x] Private IPC crate has a Unix socket with bounded framed status messages.
- [x] `fuse-promised --foreground` serves the private status IPC socket.
- [x] Private IPC can register and unregister daemon-owned provider sessions.
- [x] Private IPC can commit metadata snapshots into the daemon-owned runtime.
- [x] Private metadata commit is gated on commit readiness and rejects disabled,
  unmounted, or mount-only daemon state before mutating runtime.
- [x] Private IPC has bounded provider read request/response message helpers.
- [x] Private IPC propagates provider disconnect on provider connection close.
- [x] `libfusepromise.so` provider registration uses private daemon IPC.
- [x] Provider read requests received by `libfusepromise.so` dispatch to the
  public C read callback.
- [x] Runtime read planning enforces provider ownership, file node type,
  provider-gone state, and EOF capping.
- [~] FUSE mount lifecycle is wired behind the daemon `fuse-mount` feature;
  the current environment still lacks `pkg-config` / libfuse3 development
  metadata for enabled-feature verification.
- [~] Feature-gated read-only FUSE callbacks are wired to runtime lookup,
  directory, and provider read routing; real mount verification remains blocked
  by the missing libfuse3 development metadata.
- [x] Runtime rejects missing, relative, non-directory, foreign-owned, or
  group/other-accessible `XDG_RUNTIME_DIR` paths.
- [x] `fpctl status` queries the daemon when connected and falls back to
  `daemon=not-connected` when disconnected.
- [x] `fp_promise_commit()` is gated on daemon commit readiness and
  `fp_materialize()` returns `FP_ERR_UNAVAILABLE` until materialize IPC exists.
- [x] Basic Rust and C header verification passes.

Baseline verification:

```sh
cargo fmt --check --all
cargo check --workspace
cargo test --workspace
cc -Iinclude -x c -fsyntax-only -
c++ -Iinclude -x c++ -fsyntax-only -
```

## Goal Chain

```text
specification
  -> daemon-owned runtime
  -> private IPC
  -> user-session FUSE mount
  -> metadata commit
  -> read-only FUSE operations
  -> provider read routing
  -> materialize
  -> ABI hardening
  -> cache/performance
  -> stable packaging
```

## Phase 0: Foundation

Goal: establish the project identity, source layout, and developer-preview
skeleton without claiming runtime behavior that does not exist yet.

- [x] Define Promise filesystem model, repository boundaries, and non-goals.
- [x] Define user-space FUSE architecture with no kernel changes.
- [x] Define public C ABI as the only supported external programming surface.
- [x] Keep daemon IPC private and replaceable.
- [x] Add initial Rust workspace and public header.
- [x] Add runtime metadata skeleton.
- [x] Add private IPC skeleton.
- [x] Add daemon and `fpctl status` skeleton.
- [x] Mark pkg-config and systemd files as placeholders until install support
  exists.
- [x] Fix first implementation dependencies, architecture ownership, and
  feature build order before expanding runtime code.

Exit criteria:

- [x] `cargo fmt --check --all` passes.
- [x] `cargo check --workspace` passes.
- [x] `cargo test --workspace` passes.
- [x] Public header can be parsed by C and C++ compilers.
- [x] `fpctl status` works both with and without a running daemon.
- [x] `docs/implementation-decisions.md` records FUSE backend, MSRV,
  dependency set, private IPC direction, daemon ownership, and implementation
  sequencing.

## Phase 1: Read-Only MVP

Goal: expose metadata-only Promise trees through a user-session FUSE mount and
serve lazy read requests through provider callbacks.

### G1.1 Daemon-Owned Runtime

- [x] Move committed Promise tree ownership fully into `fuse-promised`.
- [x] Ensure daemon allocates visible promise ids, runtime node ids, and inode
  numbers.
- [x] Keep client-side `libfusepromise.so` state limited to opaque handles,
  provider callbacks, builder state before commit, and private IPC connection
  state.
- [x] Remove or quarantine any client-side in-process runtime path that could
  appear authoritative.

Acceptance:

- `fp_promise_commit()` cannot commit into a client-local namespace.
- Restarting or disconnecting a client cannot create daemon-invisible committed
  promises.
- Daemon status reports provider and promise counts from the authoritative
  runtime.

### G1.2 Private IPC Expansion

- [x] Replace status-only line protocol with a bounded private protocol.
- [x] Add handshake/version negotiation.
- [x] Add max message size checks.
- [x] Add Unix peer credential validation where available.
- [x] Add provider register/unregister messages.
- [x] Add Promise commit request/response messages.
- [x] Add provider read request/response messages.
- [x] Add provider disconnect propagation.
- [x] Keep all IPC types private to internal crates.

Acceptance:

- Public C ABI does not expose private IPC structs or message names.
- Invalid version, oversize message, bad owner, invalid path, and invalid read
  range are rejected before daemon state mutates.
- `fpctl status` continues to work through the same private runtime path.

Suggested verification:

```sh
cargo test -p fuse-promise-ipc --locked
```

### G1.3 Provider Session Routing

- [x] Register providers through `libfusepromise.so` and private IPC.
- [x] Keep a live provider session table in the daemon.
- [x] Route daemon read requests back to the provider process.
- [x] Dispatch read requests to the provider's public C callback inside the
  provider process.
- [x] Enforce provider ownership for committed trees and read requests.
- [x] Mark non-materialized and non-cached promises as provider-gone when the
  provider disconnects.

Acceptance:

- Provider lifecycle is at least `live -> disconnected`.
- Provider cannot satisfy or mutate promises it does not own.
- Provider disconnect produces deterministic read/materialize errors.

### G1.4 Metadata Commit

- [x] Serialize builder metadata through private IPC.
- [x] Validate normalized relative paths in the daemon.
- [x] Validate node type, permission bits, file size, mtime, duplicate paths,
  and parent directories.
- [x] Commit static snapshot Promise trees.
- [ ] Return a visible path under `$XDG_RUNTIME_DIR/fuse-promise/`.

Acceptance:

- `fp_promise_commit()` returns `FP_OK` and a real mounted path only after the
  daemon and FUSE mount are ready.
- `ls`, `stat`, and directory traversal do not request file bytes.
- Invalid paths such as absolute paths, `..`, NUL, and duplicate nodes fail.

### G1.5 User-Session FUSE Mount

- [x] Pick and document the internal FUSE backend.
- [~] Mount `$XDG_RUNTIME_DIR/fuse-promise/`.
- [x] Fail explicitly if `XDG_RUNTIME_DIR` is missing, not absolute, unsafe, or
  not owned by the current user.
- [~] Cleanly unmount on daemon exit.
- [x] Keep mount lifecycle user-session scoped.

Acceptance:

```sh
mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"
fpctl status
```

`fpctl status` reports mount state from the daemon.

### G1.6 Read-Only FUSE Operations

- [~] Implement `lookup`.
- [~] Implement `getattr`.
- [~] Implement `readdir`.
- [~] Implement `open`.
- [~] Implement offset-based `read`.
- [~] Implement `release`.
- [~] Map runtime failures to deterministic `errno` values.

Acceptance:

```sh
stat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file"
ls -la "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>"
cat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file"
dd if="$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file" bs=1 skip=10 count=20
```

Reads request only the byte ranges needed by the caller.

### G1.7 Read-Only MVP Gate

- [ ] A provider can create a promised directory tree.
- [ ] The tree appears under the user-session FUSE mount.
- [ ] `ls`, `stat`, and traversal work without content transfer.
- [ ] `cat` and `cp` trigger provider lazy reads.
- [ ] Provider disconnect behavior is deterministic.
- [ ] `fpctl status` and minimal inspection commands report daemon state.

## Phase 2: Materialize

Goal: copy promised files and directory subtrees into real local storage using
the same read path as lazy filesystem reads.

### G2.1 File Materialize

- [ ] Add materialize IPC request/response.
- [ ] Implement file materialize in the daemon.
- [ ] Stream provider bytes in chunks.
- [ ] Apply file mode and mtime.
- [ ] Record materialized node state.

Acceptance:

```sh
fpctl materialize <promise-file> <target-dir>
cmp <expected-file> <target-dir>/<file>
```

### G2.2 Directory Materialize

- [ ] Walk Promise subtrees recursively.
- [ ] Create directories.
- [ ] Materialize child files.
- [ ] Apply directory metadata.
- [ ] Report partial failure with structured results.

Acceptance:

```sh
fpctl materialize <promise-dir> <target-dir>
diff -r <expected-dir> <target-dir>/<dir>
```

### G2.3 Conflict, Progress, and Cancellation

- [ ] Implement `FP_CONFLICT_FAIL`.
- [ ] Implement `FP_CONFLICT_OVERWRITE`.
- [ ] Implement `FP_CONFLICT_RENAME`.
- [ ] Add progress reporting.
- [ ] Add cancellation.
- [ ] Map cancellation to public `FP_ERR_CANCELLED` and filesystem
  `ECANCELED` where applicable.

Acceptance:

- Existing target behavior is deterministic and tested.
- Cancelled jobs leave documented partial state.
- Materialized reads can use the real local path when policy allows it.

## Phase 3: ABI Hardening and Developer Release

Goal: prepare a first unstable developer ABI that downstream consumers can
build against.

### G3.1 ABI Layout Tests

- [ ] Test header/Rust constant consistency.
- [ ] Test struct sizes, alignment, and field offsets.
- [ ] Test public status and policy values.
- [ ] Test public symbol exports only expose intended `fp_` functions.
- [ ] Test panic safety for every public entrypoint.
- [ ] Test null pointer, bad `struct_size`, and version mismatch behavior.

Suggested verification:

```sh
cargo build -p fuse-promise-ffi --locked
nm -D --defined-only target/debug/libfusepromise.so
```

### G3.2 Public Error Documentation

- [ ] Document every `fp_status_t` value.
- [ ] Document filesystem `errno` mappings.
- [ ] Document provider disconnect behavior.
- [ ] Document timeout and cancellation behavior.

### G3.3 Public C Examples

- [ ] Add a minimal C provider example.
- [ ] Add a C materialize example after Phase 2.
- [ ] Ensure examples include only `fuse-promise/fuse-promise.h`.
- [ ] Ensure examples link only through `pkg-config --cflags --libs
  fuse-promise`.

### G3.4 Install Metadata

- [ ] Generate `fuse-promise.pc` from `pkgconfig/fuse-promise.pc.in`.
- [ ] Install public header.
- [ ] Install `libfusepromise.so`.
- [ ] Define soname/version policy.
- [ ] Install `fuse-promised` and `fpctl`.
- [ ] Keep systemd user service aligned with install paths.

Acceptance:

```sh
pkg-config --cflags --libs fuse-promise
cc example.c $(pkg-config --cflags --libs fuse-promise)
```

## Phase 4: Cache and Performance

Goal: improve performance without changing visible Promise filesystem
semantics.

- [ ] Keep default no-cache behavior explicit.
- [ ] Add optional read-through chunk cache.
- [ ] Track complete and incomplete byte ranges.
- [ ] Add sequential prefetch.
- [ ] Add read coalescing.
- [ ] Add materialized-file passthrough.
- [ ] Stress test large trees.
- [ ] Stress test large files and random reads.

Acceptance:

- Random reads do not require full-file download.
- Large tree creation remains metadata-only.
- Materialize streams data without holding whole files in memory.
- Provider-gone reads succeed only for complete cached or materialized content.

## Phase 5: Stable System Component

Goal: release a distribution-friendly Linux user-session component with a
stable ABI.

### G5.1 Security Model

- [ ] Document user-session isolation.
- [ ] Validate `XDG_RUNTIME_DIR` ownership and permissions.
- [ ] Validate control socket ownership and type.
- [ ] Validate provider ownership for all daemon mutations.
- [ ] Validate paths and prevent path traversal.
- [ ] Validate materialize targets and symlink behavior.
- [ ] Validate read ranges and message sizes.

### G5.2 Compatibility Matrix

- [ ] `open`
- [ ] `stat`
- [ ] `readdir`
- [ ] `read`
- [ ] `pread`
- [ ] `ls`
- [ ] `find`
- [ ] `cat`
- [ ] `cp`
- [ ] `tar`
- [ ] `rsync`

Acceptance:

- Metadata-only operations do not transfer content.
- Read-oriented tools trigger normal lazy reads.
- Tools that force full reads behave according to normal filesystem semantics.

### G5.3 Packaging

- [ ] Package public header, shared library, pkg-config file, daemon, CLI, and
  user service.
- [ ] Provide distribution packaging guidelines.
- [ ] Provide build instructions.
- [ ] Provide changelog and release notes.
- [ ] Document visible filesystem layout.
- [ ] Prepare first stable ABI release.

## Cross-Cutting Verification Gates

Run these before merging implementation milestones:

```sh
cargo fmt --check --all
cargo check --workspace --locked
cargo test --workspace --locked
git diff --check
```

Run these before ABI or release milestones:

```sh
cc -Iinclude -Wall -Wextra -fsyntax-only sample.c
c++ -Iinclude -Wall -Wextra -fsyntax-only sample.cc
nm -D --defined-only target/debug/libfusepromise.so
pkg-config --cflags --libs fuse-promise
```

Run these before FUSE milestones:

```sh
pkg-config --exists fuse3
test -e /dev/fuse
which fusermount3
XDG_RUNTIME_DIR="$(mktemp -d)" cargo run -p fuse-promise-daemon -- --foreground
fpctl status
mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"
stat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>"
cat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/<file>"
fusermount3 -u "$XDG_RUNTIME_DIR/fuse-promise"
```

## Out of Scope for This Repository

- Clipboard synchronization products.
- Desktop drag-and-drop adapters.
- Cloud provider integrations.
- P2P file transfer products.
- Desktop-environment-specific plugins.

Those projects should be separate consumers of the public C ABI.
