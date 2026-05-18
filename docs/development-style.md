# Development Style

## Intent

This document defines the codebase shape expected for `fuse-promise`.

The project should resemble a Linux user-space infrastructure component: clear public headers, stable ABI discipline, small tools, explicit runtime boundaries, predictable errors, and conservative dependencies.

## Language Policy

Use Rust for the implementation unless there is a specific reason not to.

Rust is appropriate for:

- Long-running daemon code.
- Provider session lifecycle.
- Metadata tree management.
- Path validation.
- Lazy read routing.
- Cache and materialize logic.
- Private IPC.
- FUSE adapter code.

Do not make Rust the public interface. The public interface is a C ABI exported by `libfusepromise.so` and declared by `fuse-promise/fuse-promise.h`.

Do not expose:

- Rust structs.
- Rust enums without `repr(C)`.
- Rust traits.
- Rust generics.
- Rust async futures.
- Tokio or async runtime handles.
- `serde` schemas as the public ABI.

FFI-facing code must use opaque handles, `#[repr(C)]` structs where needed, stable integer types, explicit ownership functions, and `fp_status_t` error values.

## Source Tree Shape

The implementation should use a layout close to:

```text
include/
  fuse-promise/
    fuse-promise.h

crates/
  fuse-promise-runtime/
  fuse-promise-ffi/
  fuse-promise-daemon/

tools/
  fpctl/

pkgconfig/
systemd/
tests/
  integration/

docs/
```

Do not add an `integrations/` directory to the core repository. Integration products should live outside this repository and consume the public ABI.

## Public Headers

Public headers must be C-compatible and installable.

Rules:

- Use `fp_` for public symbols.
- Use opaque handle types for runtime objects.
- Use fixed-width integer types for ABI-visible status and policy values.
- Use native `size_t` only where the ABI is intentionally native-platform
  scoped, such as in-process buffer lengths.
- Include `struct_size` in extensible public structs.
- Avoid exposing implementation language details.
- Avoid exposing daemon IPC structures.
- Keep comments factual and suitable for generated API documentation.

## Error Style

The public library should return `fp_status_t`.

Filesystem paths should surface failures through normal `errno` mappings.

The runtime should avoid stringly-typed control flow. Human-readable messages are useful for logs, but public behavior must be driven by typed status values.

## Dependency Policy

Core code should keep dependencies narrow.

Allowed dependency categories:

- FUSE interface library.
- C runtime / POSIX APIs.
- Small build-time tooling.
- Test-only frameworks.

Dependencies that imply application policy should stay out of core:

- Clipboard libraries.
- GUI toolkits.
- Cloud SDKs.
- P2P frameworks.
- Desktop-environment-specific APIs.

## Daemon Style

`fuse-promised` should be boring and inspectable.

Expected behavior:

- Start in foreground for debugging.
- Support systemd user-service operation.
- Log structured, concise runtime events.
- Fail explicitly when `$XDG_RUNTIME_DIR` is unavailable.
- Cleanly unmount on shutdown.
- Treat provider disconnect as a normal runtime event.

## CLI Style

`fpctl` is an administrative and debugging tool, not the main API.

Expected commands may include:

```text
fpctl status
fpctl list
fpctl inspect <promise-path>
fpctl materialize [--progress] [--overwrite|--rename] <promise-path> <target-dir>
fpctl destroy <promise-path>
```

The CLI should call the same public or internal runtime paths as real users. It should not become a separate implementation.

## Test Style

Tests should cover behavior visible through both the public ABI and the mounted filesystem.

Important test areas:

- Metadata-only tree commit.
- Directory enumeration.
- Offset reads.
- Short reads.
- Provider disconnect.
- Header/Rust ABI layout consistency.
- Provider callback buffer bounds.
- Materialize success.
- Materialize partial failure.
- Path normalization.
- Permission behavior.
- Daemon restart behavior.

## Documentation Style

Documentation should be direct and distribution-friendly.

Prefer:

- About.
- Status.
- Supported platforms.
- Build instructions.
- Security model.
- ABI policy.
- Examples.

Avoid marketing language and application-specific promises.
