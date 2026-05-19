# Release Notes: 1.0.3 Ubuntu 18.04 Compatibility Packages

Released 2026-05-19.

This release keeps the same stable public C ABI commitments as `1.0.0`,
`1.0.1`, and `1.0.2`:

- Public C header: `fuse-promise/fuse-promise.h`.
- Stable soname-major: `libfusepromise.so.1`.
- Public `fp_` symbols from `libfusepromise.so`.
- Installed binary names: `fuse-promised` and `fpctl`.
- Private daemon IPC and Rust crate APIs remain internal.

## Compatibility Change

Release packages are now built inside an `ubuntu:18.04` container. This lowers
the GNU/Linux glibc compatibility floor for packaged binaries to `GLIBC_2.27`.

The release package job verifies the maximum referenced `GLIBC_*` symbol for:

- `fuse-promised`
- `fpctl`
- `libfusepromise.so.<version>`

If any packaged binary requires a glibc symbol newer than `GLIBC_2.27`, the
release job fails before publishing assets.

Ubuntu 18.04 does not ship `libfuse3-dev`. For the `fuse3-promise` package,
the release build compiles against upstream libfuse `3.18.2` inside the bionic
container using the verified source archive:

```text
fuse-3.18.2.tar.gz
sha256:f01de85717e20adf5f98aff324acd85dd73d61a5ca3834d573dcf0bd6e54a298
```

The `fuse3-promise` package still depends on a runtime `libfuse3` provider on
the target system. The glibc floor is lowered for the `fuse-promise` binaries;
it does not vendor libfuse3 into the package.

## Packaging

- FUSE2 package name: `fuse-promise`.
- FUSE3 package name: `fuse3-promise`.
- Both package variants install the same public ABI, binaries, library, and
  systemd user service paths.
- The FUSE2 and FUSE3 packages conflict with each other because they install
  the same files.
- Cloudsmith defaults now target `ubuntu/bionic` for DEB repository metadata and
  `el/8` for RPM repository metadata.

## Artifacts

The release publishes the following GitHub Release assets:

```text
fuse3-promise_1.0.3-1_amd64.deb
fuse3-promise_1.0.3-1_arm64.deb
fuse3-promise-1.0.3-1.x86_64.rpm
fuse3-promise-1.0.3-1.aarch64.rpm
fuse-promise_1.0.3-1_amd64.deb
fuse-promise_1.0.3-1_arm64.deb
fuse-promise-1.0.3-1.x86_64.rpm
fuse-promise-1.0.3-1.aarch64.rpm
fuse-promise-1.0.3.tar.gz
SHA256SUMS
```
