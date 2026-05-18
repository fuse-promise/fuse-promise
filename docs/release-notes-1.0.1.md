# Release Notes: 1.0.1 Packaging Release

Released 2026-05-18.

This release keeps the same stable public C ABI commitments as `1.0.0`:

- Public C header: `fuse-promise/fuse-promise.h`.
- Stable soname-major: `libfusepromise.so.1`.
- Public `fp_` symbols from `libfusepromise.so`.
- Private daemon IPC and Rust crate APIs remain internal.

## Packaging

- Adds GitHub Actions release automation.
- Publishes native Linux packages for `amd64`/`x86_64` and `arm64`/`aarch64`.
- Adds a formal source tarball and `SHA256SUMS`.
- Keeps public package repository publishing gated on mounted FUSE validation.

## Artifacts

```text
fuse-promise_1.0.1-1_amd64.deb
fuse-promise_1.0.1-1_arm64.deb
fuse-promise-1.0.1-1.x86_64.rpm
fuse-promise-1.0.1-1.aarch64.rpm
fuse-promise-1.0.1.tar.gz
SHA256SUMS
```
