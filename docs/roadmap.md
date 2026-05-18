# Roadmap

For the detailed goal checklist with acceptance criteria, see
[Progress Goals](progress.md).

For fixed implementation dependency and sequencing decisions, see
[Implementation Decisions](implementation-decisions.md).

## Phase 0: Specification

- Define the Promise model.
- Define repository boundaries.
- Define the initial public C ABI.
- Define daemon lifecycle.
- Define read and materialize semantics.
- Add initial source layout, public header, Rust runtime skeleton, C ABI entry
  points, daemon entry point, and `fpctl status`.
- Fix the first implementation dependency set, crate ownership boundaries, and
  framework build order before expanding runtime logic.
- Add developer install metadata that generates pkg-config and systemd user
  service files from templates.

## Phase 1: Read-Only MVP

- Make `fuse-promised` the sole owner of runtime metadata, provider sessions,
  inode allocation, and mount state.
- Extend the private library-to-daemon IPC beyond the current status command.
- Make `libfusepromise.so` connect to or start the daemon instead of owning an
  independent in-process runtime.
- Make `fpctl` query the daemon through the same private runtime path.
- Implement the user-session daemon.
- Mount a FUSE filesystem under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Support static Promise trees.
- Implement `getattr`, `readdir`, `open`, and `read`.
- Implement provider read callbacks.
- Provide a minimal `fpctl` inspection command.

## Phase 2: Materialize

- Implement file materialization.
- Implement recursive directory materialization.
- Add conflict policies.
- Add progress and cancellation.
- Add materialized node state.

## Phase 3: ABI Hardening

- Freeze `fuse-promise.h` for a first unstable developer release.
- Add ABI layout tests for header/Rust constants, struct sizes, alignment, and
  field offsets.
- Add pkg-config generation and install metadata.
- Add public error documentation.
- Add C examples that use only the public API.

## Phase 4: Cache and Performance

- Make the default no-cache policy explicit.
- Add optional chunk cache.
- Add sequential prefetch policy.
- Add read coalescing.
- Add materialized-file passthrough.
- Add stress tests for large trees and large files.

## Phase 5: Stable System Component

- Package user service files.
- Document security model.
- Document distribution packaging guidelines.
- Add compatibility tests with common Linux command-line tools.
- Prepare the first stable ABI release.

## Explicitly Out of Tree

The following should be built as separate projects or downstream users:

- Remote clipboard tools.
- Desktop drag-and-drop adapters.
- Cloud storage providers.
- P2P file transfer products.
- Desktop-environment-specific plugins.
