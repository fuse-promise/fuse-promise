# Release Notes: 1.0.2 FUSE Backend Packages

Released 2026-05-19.

This release keeps the same stable public C ABI commitments as `1.0.0` and
`1.0.1`:

- Public C header: `fuse-promise/fuse-promise.h`.
- Stable soname-major: `libfusepromise.so.1`.
- Public `fp_` symbols from `libfusepromise.so`.
- Installed binary names: `fuse-promised` and `fpctl`.
- Private daemon IPC and Rust crate APIs remain internal.

## Packaging

- Adds separate FUSE2 and FUSE3 package variants.
- FUSE2 package name: `fuse-promise`.
- FUSE3 package name: `fuse3-promise`.
- Both package variants install the same public ABI, binaries, library, and
  systemd user service paths.
- The FUSE2 and FUSE3 packages conflict with each other because they install
  the same files.
- Release builds now validate both daemon backend features.

## Documentation

- Adds a minimal provider example covering directory/file attributes,
  provider-backed reads, and materialize.
- Documents selectable FUSE2/FUSE3 build, smoke-test, install, and package
  commands.
- Updates the project motivation to reference Windows `IDataObject` /
  `IStream` and macOS file promises.

## Artifacts

```text
fuse3-promise_1.0.2-1_amd64.deb
fuse3-promise_1.0.2-1_arm64.deb
fuse3-promise-1.0.2-1.x86_64.rpm
fuse3-promise-1.0.2-1.aarch64.rpm
fuse-promise_1.0.2-1_amd64.deb
fuse-promise_1.0.2-1_arm64.deb
fuse-promise-1.0.2-1.x86_64.rpm
fuse-promise-1.0.2-1.aarch64.rpm
fuse-promise-1.0.2.tar.gz
SHA256SUMS
```
