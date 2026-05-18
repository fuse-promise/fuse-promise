# Stable ABI Release Notes Draft

These notes define the first stable ABI release candidate. The final version
and date are set only after every gate in `docs/stable-abi-release.md` passes
against the exact release artifact.

## Stable Commitments

- Public C header: `fuse-promise/fuse-promise.h`.
- Exported `fp_` symbols from `libfusepromise.so`.
- Stable soname-major: `libfusepromise.so.1`.
- Generated `fuse-promise.pc` metadata.
- Installed binary names: `fuse-promised` and `fpctl`.
- Default user-session mount layout:
  `$XDG_RUNTIME_DIR/fuse-promise/<promise-id>`.
- Public status values already defined by `fp_status_t`; existing numeric
  values must not be renumbered.
- Public materialize conflict policies already defined by
  `fp_conflict_policy_t`; existing numeric values must not be renumbered.
- File and directory materialize with fail-on-conflict, overwrite, and rename.
- Materialize progress callback layout and progress-callback cancellation with
  `FP_ERR_CANCELLED`.
- Provider read callback request/response layout and buffer ownership rules.
- Opaque handle ownership functions and null-pointer behavior.

## Expansion Rules

- Public structs keep `struct_size`; new trailing fields may be added only with
  documented default behavior for older callers.
- New status and policy values may be added, but existing values must not be
  renumbered.
- The first stable ABI keeps NUL-terminated UTF-8 path strings. Future Linux
  byte-path APIs may be added as separate additive entrypoints.
- Public function names remain stable after this release.
- Internal daemon IPC, Rust crate APIs, provider routing internals, cache
  implementation details, and the FUSE backend crate remain private.

## Verification

The stable release artifact must pass:

```sh
tests/stable-release-gates.sh
```

The FUSE gates require libfuse3 development metadata, `/dev/fuse`, and
`fusermount3`.
