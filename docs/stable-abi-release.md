# Stable ABI Release Readiness

This document tracks the work required before `fuse-promise` can declare a
stable public ABI release. The private Rust crates and daemon IPC remain
internal implementation details; the stable surface is the installed C ABI and
observable filesystem behavior.

## Stable Surface

The stable release candidate must freeze:

- `include/fuse-promise/fuse-promise.h`
- exported `fp_` symbols from `libfusepromise.so`
- `libfusepromise.so.1` soname policy
- generated `fuse-promise.pc`
- documented `fp_status_t` values
- documented `fp_conflict_policy_t` values
- installed daemon and CLI names
- default user-session mount layout
- materialize conflict, progress, and progress-callback cancellation behavior

The stable release must not freeze:

- Rust crate APIs
- daemon private IPC message types
- provider routing internals
- cache implementation details
- FUSE backend crate choice

## Required Gates

Run these before declaring the ABI stable:

```sh
tests/stable-release-gates.sh
```

`tests/stable-release-gates.sh` defaults to `BUILD_PROFILE=release` and
`SONAME_MAJOR=1`, then runs the workspace, ABI, install, FUSE, cache,
performance, security, materialize, and whitespace gates.

The FUSE gates require libfuse3 development metadata, `/dev/fuse`, and
`fusermount3`.

## Release Blockers

The first stable ABI release remains blocked until these are resolved:

- Re-run ABI hardening against the exact release build artifact with
  `BUILD_PROFILE=release SONAME_MAJOR=1 tests/abi-hardening.sh`.
- Run the full stable release gate with `tests/stable-release-gates.sh`.
- Set the final stable release version and date in the changelog and stable
  release notes.
- Tag the release only after the installed header, pkg-config metadata, soname,
  CLI behavior, release notes, and smoke gates match this document.

## Stable Candidate Decisions

- The public header exposes only handles and structs used by callable public
  functions; unused future job handles are not part of the developer-preview
  ABI surface.
- The first stable ABI release will use soname-major `1`; developer-preview
  builds continue to default to soname-major `0`.
- `FP_CONFLICT_FAIL`, `FP_CONFLICT_OVERWRITE`, `FP_CONFLICT_RENAME`,
  `fp_materialize_progress_t`, and progress-callback cancellation are stable
  candidate commitments for the first stable ABI release.

## Versioning Rule

Until this checklist is complete, `0.1.0` remains a developer-preview release.
Its developer-preview notes live in `docs/release-notes-0.1.0.md`.
The stable release notes draft lives in `docs/release-notes-stable.md`; set the
final stable release version and date only after the gate passes.
