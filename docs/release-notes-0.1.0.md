# Release Notes: 0.1.0 Developer Preview

`0.1.0` is a developer-preview release for downstream build, packaging, and
integration experiments. It does not declare the public ABI stable.

## Scope

- Public C ABI in `fuse-promise/fuse-promise.h`.
- `libfusepromise.so` with developer soname-major `0`.
- `fuse-promised` user-session daemon.
- `fpctl status`, `fpctl list`, and `fpctl materialize`.
- User-session FUSE mount under `$XDG_RUNTIME_DIR/fuse-promise/`.
- Provider registration through private daemon IPC.
- Metadata commit into the daemon-owned runtime.
- Lazy FUSE reads routed to provider callbacks.
- File and directory materialize.
- Materialize conflict policies: fail, overwrite, and rename.
- Materialize progress callback and progress-callback cancellation.
- Explicit default no-cache policy.
- Optional read-through cache with range tracking, read coalescing, and
  sequential prefetch.
- Materialized-file read passthrough after provider disconnect.
- Developer install metadata for the public header, shared library, pkg-config
  file, daemon, CLI, and systemd user service template.

## Stability Statement

The public header, status values, conflict policy values, exported `fp_`
symbols, pkg-config metadata, installed binary names, and default mount layout
are tested as a developer-preview ABI. They may still change before the first
stable ABI release.

The stable ABI release remains gated on:

- Final stable release version and date after the gate passes.
- Stable releases use soname-major `1`; this developer-preview release keeps
  soname-major `0`.
- ABI hardening against the exact release build artifact.
- Full FUSE, cache, performance, security, install, and ABI verification gates.

## Not Included

- Stable ABI guarantee.
- Network, cloud-provider, P2P, clipboard, or desktop integration logic.
- Cross-user isolation beyond the local Unix user-session boundary.
- Application-specific provider policy.

## Verification

The developer-preview release candidate should pass:

```sh
cargo fmt --check --all
cargo check --workspace --locked
cargo test --workspace --locked
BUILD_PROFILE=release tests/abi-hardening.sh
BUILD_PROFILE=release tests/install-metadata.sh
tests/read-only-mvp-smoke.sh
tests/read-through-cache-smoke.sh
tests/performance-stress.sh
tests/control-socket-security.sh
tests/materialize-security.sh
git diff --check
```
