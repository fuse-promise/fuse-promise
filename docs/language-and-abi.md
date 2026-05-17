# Language and ABI

## Decision

`fuse-promise` should be implemented primarily in Rust.

The public interface must remain a stable C ABI.

```text
implementation language: Rust
public ABI: C
public header: fuse-promise/fuse-promise.h
shared library: libfusepromise.so
daemon: fuse-promised
administrative CLI: fpctl
```

This is a deliberate split. Rust is used to build the system safely. C ABI is used to make the system consumable by the Linux ecosystem.

## Why Rust

`fuse-promise` has several implementation areas where Rust is a good fit:

- Long-running daemon state.
- Provider session lifecycle.
- Ownership and disconnect handling.
- Lazy read routing.
- Offset and bounds validation.
- Metadata tree management.
- Path normalization.
- Cache state.
- Materialize jobs.
- Private IPC.
- Concurrent request handling.

These areas involve external input, asynchronous events, process lifetime, and filesystem-visible errors. Rust helps keep those concerns explicit.

## Why Not Rust ABI

Rust is not suitable as the public ABI for this project.

The public users of `fuse-promise` may be written in C, C++, Go, Python, Rust, Qt, GTK, or other languages. They need a stable system interface that distributions can package and downstream applications can bind to.

The project must not require public consumers to depend on:

- Rust crate APIs.
- Rust ABI stability.
- Rust traits.
- Rust generics.
- Rust ownership types.
- Rust async futures.
- Tokio or another async runtime.
- Cargo as the only consumption path.

Rust crates may exist for internal organization or optional language bindings, but they are not the primary system interface.

## Public Contract

The public contract is:

```text
/usr/include/fuse-promise/fuse-promise.h
/usr/lib/libfusepromise.so
/usr/lib/pkgconfig/fuse-promise.pc
```

Public functions use the `fp_` prefix.

Public object references are opaque handles:

```c
typedef struct fp_context fp_context_t;
typedef struct fp_provider fp_provider_t;
typedef struct fp_promise_builder fp_promise_builder_t;
```

Public errors use explicit status values:

```c
typedef uint32_t fp_status_t;
```

Extensible public structs should include a `struct_size` field.

## FFI Rules

FFI-facing Rust code must follow strict ABI discipline:

- Use `extern "C"` for exported functions.
- Use `#[repr(C)]` for ABI-visible structs and enums.
- Use opaque pointers for runtime-owned objects.
- Provide explicit create and destroy functions.
- Do not let Rust panics cross the FFI boundary.
- Do not expose borrowed Rust references.
- Do not expose Rust `String`, `Vec`, `PathBuf`, `Result`, or `Option` directly.
- Do not expose async functions directly.
- Convert all errors into `fp_status_t`.

## Internal Rust Structure

The Rust implementation may be organized into internal crates or modules such as:

```text
runtime
daemon
fuse
materialize
cache
ipc
ffi
tools
```

The internal module layout may change without breaking the public ABI.

## FUSE Layer

The FUSE adapter may use a Rust FUSE library or bind to libfuse/libfuse3.

The chosen FUSE backend is an implementation detail. Public users must not know whether the daemon uses `libfuse`, `fuser`, `fuse3`, or a future adapter.

The stable behavior is the mounted filesystem and the public C ABI, not the internal FUSE crate.

## Async Policy

The implementation may use async internally.

Async must not appear in the public C ABI. Provider callbacks exposed through the C ABI should be plain function pointers or handle-based polling/job APIs. If the runtime needs async behavior, `libfusepromise.so` and `fuse-promised` must bridge that internally.

## Binding Policy

Language bindings may be added later.

All official bindings should bind to the C ABI instead of depending on private Rust internals. This keeps the ABI surface small and keeps the implementation free to evolve.
