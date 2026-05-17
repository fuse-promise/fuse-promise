# Roadmap

## Phase 0: Specification

- Define the Promise model.
- Define repository boundaries.
- Define the initial public C ABI.
- Define daemon lifecycle.
- Define read and materialize semantics.

## Phase 1: Read-Only MVP

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
- Add ABI version checks.
- Add pkg-config metadata.
- Add public error documentation.
- Add C examples that use only the public API.

## Phase 4: Cache and Performance

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

