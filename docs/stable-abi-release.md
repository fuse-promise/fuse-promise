# Stable ABI Release Readiness

This document tracks the work required before `fuse-promise` can declare a
stable public ABI release. The private Rust crates and daemon IPC remain
internal implementation details; the stable surface is the installed C ABI and
observable filesystem behavior.

## Stable Surface

The stable release candidate must freeze:

- `include/fuse-promise/fuse-promise.h`
- exported `fp_` symbols from `libfusepromise.so`
- `libfusepromise.so.0` soname policy
- generated `fuse-promise.pc`
- documented `fp_status_t` values
- documented `fp_conflict_policy_t` values
- installed daemon and CLI names
- default user-session mount layout

The stable release must not freeze:

- Rust crate APIs
- daemon private IPC message types
- provider routing internals
- cache implementation details
- FUSE backend crate choice

## Required Gates

Run these before declaring the ABI stable:

```sh
cargo fmt --check --all
cargo check --workspace --locked
cargo test --workspace --locked
tests/abi-hardening.sh
tests/install-metadata.sh
tests/read-only-mvp-smoke.sh
tests/read-through-cache-smoke.sh
tests/performance-stress.sh
tests/control-socket-security.sh
tests/materialize-security.sh
git diff --check
```

The FUSE gates require libfuse3 development metadata, `/dev/fuse`, and
`fusermount3`.

## Release Blockers

The first stable ABI release remains blocked until these are resolved:

- Decide whether the implemented materialize conflict policies are stable
  public ABI commitments.
- Decide whether the progress callback and progress-callback cancellation are
  stable public ABI commitments.
- Reconcile `CHANGELOG.md` with the chosen stability statement.
- Re-run ABI hardening against the exact release build artifact.
- Tag the release only after the installed header, pkg-config metadata, soname,
  CLI behavior, and smoke gates match this document.

## Versioning Rule

Until this checklist is complete, `0.1.0` remains a developer-preview release.
After the checklist is complete, the release notes must state which public ABI
elements are stable and which values are reserved for future expansion.
