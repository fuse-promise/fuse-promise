# Changelog

## 0.1.0 - Unreleased

Initial developer release scope:

- Public C ABI in `fuse-promise/fuse-promise.h`.
- `libfusepromise.so` with `libfusepromise.so.0` soname policy.
- Private Unix socket IPC between public clients and `fuse-promised`.
- User-session FUSE mount under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Provider registration, metadata commit, lazy read routing, and provider-gone
  read errors.
- File and directory materialize with fail-on-conflict, overwrite, and rename
  behavior.
- Explicit default `no-cache` policy and opt-in read-through cache with range
  tracking, sequential prefetch, and read coalescing.
- Materialized-file read passthrough after provider disconnect.
- `fpctl status`, `fpctl list`, and `fpctl materialize`.
- pkg-config metadata, install script, and systemd user service template.

Not included in this release:

- Stable ABI guarantee.
- Progress reporting or cancellation.
- Network, cloud-provider, P2P, clipboard, or desktop integration logic.
