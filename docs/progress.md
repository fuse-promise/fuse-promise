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
- [x] FUSE mount lifecycle is wired behind the daemon `fuse-mount` feature and
  has been verified with libfuse3 development metadata available.
- [x] Feature-gated read-only FUSE callbacks are wired to runtime lookup,
  directory, and provider read routing, and have been verified against a
  mounted committed tree.
- [x] Runtime rejects missing, relative, non-directory, foreign-owned, or
  group/other-accessible `XDG_RUNTIME_DIR` paths.
- [x] `fpctl status` queries the daemon when connected and falls back to
  `daemon=not-connected` when disconnected.
- [x] `fpctl list` reports daemon-owned providers, promises, and runtime nodes
  through private IPC.
- [x] `fp_promise_commit()` is gated on daemon commit readiness.
- [x] `fp_materialize()` and `fpctl materialize` can materialize files and
  directory subtrees through private daemon IPC using the provider read path.
- [x] `fp_promise_commit()` has FFI coverage for the commit-ready success path
  returning `$XDG_RUNTIME_DIR/fuse-promise/<promise-id>`.
- [x] Public ABI verifier covers header constants/layout, exported symbols,
  panic boundaries, invalid arguments, generated pkg-config metadata, and C
  example linking.

Baseline verification:

```sh
cargo fmt --check --all
cargo check --workspace
cargo test --workspace
tests/abi-hardening.sh
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

## Dependency and Architecture Gate

- [x] Direct implementation dependency set is documented and frozen for Phase 1.
- [x] FUSE backend is fixed to `fuser` over libfuse3/fusermount3, behind the
  daemon `fuse-mount` feature.
- [x] Default workspace build remains independent of `pkg-config` and libfuse3
  development metadata.
- [x] No async runtime, cache library, database, HTTP, cloud SDK, desktop
  integration, or generated-header dependency is planned for the read-only MVP.
- [x] Crate ownership boundaries are fixed before further runtime logic work.

Do not add a new dependency or widen a crate boundary while implementing a goal
unless `docs/implementation-decisions.md` is updated first.

## Immediate Implementation Queue

This queue is the current goal list for turning the framework into behavior.
Each row should become one focused implementation loop with one verification
pass and one pushable commit.

| Status | Order | Goal | Primary Scope | Exit Check |
|---|---:|---|---|---|
| [x] | 1 | Freeze MVP visible layout | Runtime, FUSE adapter, CLI docs, and tests. | The root directory and `$XDG_RUNTIME_DIR/fuse-promise/<promise-id>` layout are documented before more FUSE behavior depends on it. |
| [x] | 2 | Finish G1.4 public commit success path | FFI test coverage over a daemon commit-ready state; no public ABI change. | `fp_promise_commit()` returns `FP_OK`, writes `$XDG_RUNTIME_DIR/fuse-promise/promise-1`, and consumes the builder only on success. |
| [x] | 3 | Verify G1.5 feature build gate | Environment and daemon feature build; no fallback to unsafe paths. | `pkg-config --exists fuse3` and `cargo check -p fuse-promise-daemon --features fuse-mount --locked` pass without stubs. |
| [x] | 4 | Verify G1.5 real mount lifecycle | Feature daemon runtime with shared `XDG_RUNTIME_DIR`. | `mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"` succeeds, `fpctl status` reports mounted state, and daemon shutdown unmounts cleanly. |
| [x] | 5 | Close G1.6 metadata-only FUSE ops | Feature-gated `lookup`, `getattr`, and `readdir` over daemon runtime. | `stat`, `ls`, and `find` work against a committed tree without provider read requests. |
| [x] | 6 | Close G1.6 FUSE read routing | Feature-gated `open`, offset `read`, `release`, and errno mapping. | `cat` and offset `dd` request only needed byte ranges and provider errors map deterministically. |
| [x] | 7 | Close G1.7 read-only MVP gate | End-to-end provider, commit, mount, inspect, lazy read, and disconnect behavior. | A provider-created tree is visible, metadata reads transfer no bytes, file reads route only requested ranges, and disconnect errors are deterministic. |
| [x] | 8 | Start G2.1 single-file materialize | Private materialize IPC plus daemon file copy using the existing read path. | `fpctl materialize <promise-file> <target-dir>` writes matching file content and metadata. |
| [x] | 9 | Add G2.2 directory materialize | Recursive tree walk, directory creation, child file materialize, metadata application. | `diff -r` matches an expected directory tree. |
| [x] | 10 | Harden G3 developer ABI | Header/constant/layout/symbol/panic tests and C examples. | Public ABI tests pass and examples link only through the public header and pkg-config metadata. |

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
- [x] Return a visible path under `$XDG_RUNTIME_DIR/fuse-promise/`; the public
  ABI success path is covered against a commit-ready daemon state and the real
  mounted path is covered by the FUSE smoke harness.

Acceptance:

- `fp_promise_commit()` returns `FP_OK` and a real mounted path only after the
  daemon and FUSE mount are ready.
- `ls`, `stat`, and directory traversal do not request file bytes.
- Invalid paths such as absolute paths, `..`, NUL, and duplicate nodes fail.

### G1.5 User-Session FUSE Mount

- [x] Pick and document the internal FUSE backend.
- [x] Mount `$XDG_RUNTIME_DIR/fuse-promise/`.
- [x] Fail explicitly if `XDG_RUNTIME_DIR` is missing, not absolute, unsafe, or
  not owned by the current user.
- [x] Cleanly unmount on daemon exit.
- [x] Keep mount lifecycle user-session scoped.

Acceptance:

```sh
mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"
fpctl status
```

`fpctl status` reports mount state from the daemon.

### G1.6 Read-Only FUSE Operations

- [x] Implement `lookup`.
- [x] Implement `getattr`.
- [x] Implement `readdir`.
- [x] Implement `open`.
- [x] Implement offset-based `read`.
- [x] Implement `release`.
- [x] Map runtime failures to deterministic `errno` values.

Acceptance:

```sh
stat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file"
ls -la "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>"
cat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file"
dd if="$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/file" bs=1 skip=10 count=20
```

Reads request only the byte ranges needed by the caller.

The repeatable FUSE smoke harness covers a public C ABI provider committing a
tree and filesystem access through `fpctl status`, `fpctl list`, `find`, `ls`,
`stat`, offset `dd`, `cat`, `cp`, file materialize, directory materialize,
provider disconnect, and provider-gone read failure:

```sh
tests/read-only-mvp-smoke.sh
tests/read-through-cache-smoke.sh
tests/performance-stress.sh
```

### G1.7 Read-Only MVP Gate

- [x] A provider can create a promised directory tree.
- [x] The tree appears under the user-session FUSE mount.
- [x] `ls`, `stat`, and traversal work without content transfer.
- [x] `cat` and `cp` trigger provider lazy reads.
- [x] Provider disconnect behavior is deterministic.
- [x] `fpctl status` and minimal inspection commands report daemon state.

## Phase 2: Materialize

Goal: copy promised files and directory subtrees into real local storage using
the same read path as lazy filesystem reads.

### G2.1 File Materialize

- [x] Add materialize IPC request/response.
- [x] Implement file materialize in the daemon.
- [x] Stream provider bytes in chunks.
- [x] Apply file mode and mtime.
- [x] Record materialized node state.

Acceptance:

```sh
fpctl materialize <promise-file> <target-dir>
cmp <expected-file> <target-dir>/<file>
```

### G2.2 Directory Materialize

- [x] Walk Promise subtrees recursively.
- [x] Create directories.
- [x] Materialize child files.
- [x] Apply directory metadata.
- [x] Report partial failure with structured results.

Acceptance:

```sh
fpctl materialize <promise-dir> <target-dir>
diff -r <expected-dir> <target-dir>/<dir>
```

### G2.3 Conflict, Progress, and Cancellation

- [x] Implement `FP_CONFLICT_FAIL`.
- [x] Implement `FP_CONFLICT_OVERWRITE`.
- [x] Implement `FP_CONFLICT_RENAME`.
- [x] Add progress reporting.
- [x] Add cancellation.
- [x] Map cancellation to public `FP_ERR_CANCELLED` and filesystem
  `ECANCELED` where applicable.

Acceptance:

- Existing target behavior is deterministic and tested.
- Cancelled jobs leave documented partial state.
- Materialized reads can use the real local path when policy allows it.

## Phase 3: ABI Hardening and Developer Release

Goal: prepare a first unstable developer ABI that downstream consumers can
build against.

### G3.1 ABI Layout Tests

- [x] Test header/Rust constant consistency.
- [x] Test struct sizes, alignment, and field offsets.
- [x] Test public status and policy values.
- [x] Test public symbol exports only expose intended `fp_` functions.
- [x] Test panic safety for every public entrypoint.
- [x] Test null pointer, bad `struct_size`, and version mismatch behavior.

Suggested verification:

```sh
cargo build -p fuse-promise-ffi --locked
nm -D --defined-only target/debug/libfusepromise.so
tests/abi-hardening.sh
```

### G3.2 Public Error Documentation

- [x] Document every `fp_status_t` value.
- [x] Document filesystem `errno` mappings.
- [x] Document provider disconnect behavior.
- [x] Document timeout and cancellation behavior.

### G3.3 Public C Examples

- [x] Add a minimal C provider example.
- [x] Add a C materialize example after Phase 2.
- [x] Ensure examples include only `fuse-promise/fuse-promise.h`.
- [x] Ensure examples link only through `pkg-config --cflags --libs
  fuse-promise`.

### G3.4 Install Metadata

- [x] Generate `fuse-promise.pc` from `pkgconfig/fuse-promise.pc.in`.
- [x] Install public header.
- [x] Install `libfusepromise.so`.
- [x] Define soname/version policy.
- [x] Install `fuse-promised` and `fpctl`.
- [x] Keep systemd user service aligned with install paths.

Acceptance:

```sh
PREFIX="$(mktemp -d)" scripts/install-dev.sh
PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig" pkg-config --cflags --libs fuse-promise
tests/install-metadata.sh
pkg-config --cflags --libs fuse-promise
cc example.c $(pkg-config --cflags --libs fuse-promise)
```

## Phase 4: Cache and Performance

Goal: improve performance without changing visible Promise filesystem
semantics.

- [x] Keep default no-cache behavior explicit.
- [x] Add optional read-through chunk cache.
- [x] Track complete and incomplete byte ranges.
- [x] Add sequential prefetch.
- [x] Add read coalescing.
- [x] Add materialized-file passthrough.
- [x] Stress test large trees.
- [x] Stress test large files and random reads.

Acceptance:

- Random reads do not require full-file download.
- Large tree creation remains metadata-only.
- Materialize streams data without holding whole files in memory.
- `fpctl status` reports `cache_policy=no-cache`.
- `fpctl status` reports `cache_policy=read-through` when the daemon is started
  with `--cache=read-through`.
- Provider-gone reads succeed only for complete materialized content or complete
  cached ranges in read-through mode.
- Read-through mode prefetches the next sequential range after full provider
  reads.
- Read-through mode coalesces provider reads to cache chunks while returning
  only the originally requested bytes to FUSE.
- `tests/performance-stress.sh` verifies metadata-only traversal over a large
  tree and bounded provider transfer for random reads from a large file.

## Phase 5: Stable System Component

Goal: release a distribution-friendly Linux user-session component with a
stable ABI.

### G5.1 Security Model

- [x] Document user-session isolation.
- [x] Validate `XDG_RUNTIME_DIR` ownership and permissions.
- [x] Validate control socket ownership and type.
- [x] Validate provider ownership for all daemon mutations.
- [x] Validate paths and prevent path traversal.
- [x] Validate materialize targets and symlink behavior.
- [x] Validate read ranges and message sizes.

Suggested verification:

```sh
cargo test -p fuse-promise-runtime --locked
cargo test -p fuse-promise-ipc --locked
tests/control-socket-security.sh
tests/materialize-security.sh
tests/read-only-mvp-smoke.sh
```

### G5.2 Compatibility Matrix

- [x] `open`
- [x] `stat`
- [x] `readdir`
- [x] `read`
- [x] `pread`
- [x] `ls`
- [x] `find`
- [x] `cat`
- [x] `cp`
- [x] `tar`
- [x] `rsync`

Acceptance:

- Metadata-only operations do not transfer content.
- Read-oriented tools trigger normal lazy reads.
- Tools that force full reads behave according to normal filesystem semantics.
- `tests/read-only-mvp-smoke.sh` covers this compatibility matrix.

### G5.3 Packaging

- [x] Package public header, shared library, pkg-config file, daemon, CLI, and
  user service.
- [x] Provide distribution packaging guidelines.
- [x] Provide build instructions.
- [x] Provide changelog.
- [x] Provide developer-preview release notes.
- [~] Prepare stable release notes.
- [x] Document visible filesystem layout.
- [~] Prepare first stable ABI release.

Suggested verification:

```sh
tests/install-metadata.sh
```

`docs/stable-abi-release.md` defines the remaining stable ABI release gates and
blockers.

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
cargo check -p fuse-promise-daemon --features fuse-mount --locked
test -e /dev/fuse
which fusermount3
export XDG_RUNTIME_DIR="$(mktemp -d)"
cargo run -p fuse-promise-daemon --features fuse-mount -- --foreground &
daemon_pid=$!
cargo run -p fpctl -- status
mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"
stat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>"
cat "$XDG_RUNTIME_DIR/fuse-promise/<promise-id>/<file>"
kill "$daemon_pid"
wait "$daemon_pid" || true
! mountpoint -q "$XDG_RUNTIME_DIR/fuse-promise"
fusermount3 -u "$XDG_RUNTIME_DIR/fuse-promise" || true
tests/read-only-mvp-smoke.sh
tests/performance-stress.sh
```

## Out of Scope for This Repository

- Clipboard synchronization products.
- Desktop drag-and-drop adapters.
- Cloud provider integrations.
- P2P file transfer products.
- Desktop-environment-specific plugins.

Those projects should be separate consumers of the public C ABI.
