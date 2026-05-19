# Release Notes: 1.0.4 Package License Metadata

Released 2026-05-19.

This release keeps the same stable public C ABI commitments as `1.0.0`,
`1.0.1`, `1.0.2`, and `1.0.3`:

- Public C header: `fuse-promise/fuse-promise.h`.
- Stable soname-major: `libfusepromise.so.1`.
- Public `fp_` symbols from `libfusepromise.so`.
- Installed binary names: `fuse-promised` and `fpctl`.
- Private daemon IPC and Rust crate APIs remain internal.

## Packaging Fix

Generated DEB and RPM packages now carry complete license metadata:

- RPM metadata sets `License: Apache-2.0`.
- Binary packages include `/usr/share/doc/<package>/copyright`.
- Binary packages include `/usr/share/licenses/<package>/LICENSE`.

The package payload, FUSE backend selection, and glibc compatibility target are
otherwise unchanged from `1.0.3`.

## Artifacts

The release publishes the following GitHub Release assets:

```text
fuse3-promise_1.0.4-1_amd64.deb
fuse3-promise_1.0.4-1_arm64.deb
fuse3-promise-1.0.4-1.x86_64.rpm
fuse3-promise-1.0.4-1.aarch64.rpm
fuse-promise_1.0.4-1_amd64.deb
fuse-promise_1.0.4-1_arm64.deb
fuse-promise-1.0.4-1.x86_64.rpm
fuse-promise-1.0.4-1.aarch64.rpm
fuse-promise-1.0.4.tar.gz
SHA256SUMS
```
