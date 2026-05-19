# Changelog

## 1.0.3 - 2026-05-19

Ubuntu 18.04 compatibility packaging update:

- Build release DEB/RPM packages inside an `ubuntu:18.04` container.
- Lower the GNU/Linux glibc compatibility floor for release binaries to
  `GLIBC_2.27`.
- Build the FUSE3 backend against upstream libfuse `3.18.2` inside the bionic
  container because Ubuntu 18.04 does not ship `libfuse3-dev`.
- Add release packaging checks that fail when packaged binaries reference
  `GLIBC_*` symbols newer than the configured compatibility floor.
- Keep both package variants:
  `fuse-promise` for FUSE2 and `fuse3-promise` for FUSE3.
- Update Cloudsmith defaults to target `ubuntu/bionic` and `el/8` package
  repository metadata.
- Keep the public C ABI and `libfusepromise.so.1` soname-major compatible.

## 1.0.2 - 2026-05-19

FUSE backend packaging update:

- Add selectable FUSE2 and FUSE3 daemon build features.
- Add FUSE2 and FUSE3 package variants:
  `fuse-promise` for FUSE2 and `fuse3-promise` for FUSE3.
- Keep installed binary names stable: `fuse-promised` and `fpctl`.
- Keep the public C ABI and `libfusepromise.so.1` soname-major compatible.
- Document the minimal provider example, including directory/file attributes,
  provider-backed reads, and materialize.
- Update README references to the `IDataObject` / `IStream` and macOS file
  promise model.

## 1.0.1 - 2026-05-18

Packaging and release automation update:

- Add GitHub Actions release automation for GitHub Release assets.
- Build DEB/RPM artifacts for `amd64`/`x86_64` and `arm64`/`aarch64`.
- Add formal source tarball and `SHA256SUMS` release assets.
- Keep Cloudsmith package repository publishing gated on mounted FUSE tests.

The public C ABI remains compatible with `1.0.0` and keeps the
`libfusepromise.so.1` soname-major.

## 1.0.0 - 2026-05-18

First stable ABI release:

- Public C ABI in `fuse-promise/fuse-promise.h`.
- `libfusepromise.so` with `libfusepromise.so.1` soname policy.
- Private Unix socket IPC between public clients and `fuse-promised`.
- User-session FUSE mount under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Provider registration, metadata commit, lazy read routing, and provider-gone
  read errors.
- File and directory materialize with fail-on-conflict, overwrite, rename,
  progress reporting, and progress-callback cancellation behavior.
- Explicit default `no-cache` policy and opt-in read-through cache with range
  tracking, sequential prefetch, and read coalescing.
- Materialized-file read passthrough after provider disconnect.
- `fpctl status`, `fpctl list`, and `fpctl materialize`.
- pkg-config metadata, install script, and systemd user service template.

Not included in this release:

- Network, cloud-provider, P2P, clipboard, or desktop integration logic.
