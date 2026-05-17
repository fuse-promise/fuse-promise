---
name: fuse-promise-system-component
description: Use when working inside the fuse-promise repository to preserve its boundary as a pure Linux user-space Promise filesystem component with a stable C ABI, FUSE runtime, daemon, and materialize semantics. Trigger for architecture, API, documentation, implementation, review, or roadmap work in this repository.
metadata:
  short-description: Maintain fuse-promise system-component boundaries
---

# fuse-promise System Component

## Core Rule

Treat `fuse-promise` as a Linux system component, not as an application.

The repository owns the generic Promise filesystem runtime, public C ABI, user-session daemon, FUSE adapter, metadata model, provider session model, cache policy, and materialize operation.

The repository must not own clipboard synchronization, cloud providers, P2P transports, desktop drag-and-drop adapters, or application-specific remote file logic.

## Public Boundary

Upper-layer software must interact through:

- `fuse-promise/fuse-promise.h`
- `libfusepromise.so`
- `pkg-config --cflags --libs fuse-promise`

Do not expose daemon IPC as public API. IPC between `libfusepromise.so` and `fuse-promised` is private and replaceable.

## Architecture Guidance

Preserve this layering:

```text
external applications
  -> public C ABI
  -> private runtime and daemon
  -> FUSE adapter
  -> Linux FUSE kernel interface
```

Default user-session mount:

```text
$XDG_RUNTIME_DIR/fuse-promise/
```

Do not use a global shared mount as the default.

## API Guidance

Prefer a stable C ABI with opaque handles and versioned structs.

Use the `fp_` prefix for public symbols.

Hide Rust, C++, or internal implementation types from the public ABI.

Provider callbacks should be expressed as public ABI concepts, but the callback transport from daemon to provider must remain internal.

## Repository Hygiene

Allowed in the core repository:

- Core runtime.
- Public header.
- Shared library implementation.
- User-session daemon.
- FUSE adapter.
- Materialize engine.
- Cache and metadata systems.
- CLI for inspection and administration.
- Tests.
- Minimal examples that only demonstrate public API usage.

Not allowed in the core repository:

- `integrations/` for business-specific adapters.
- Clipboard products.
- Cloud-specific providers.
- P2P transfer implementations.
- Desktop-environment plugins.

If a task asks for one of those features, design it as an external consumer of the public API unless the user explicitly changes the repository scope.

## Documentation Guidance

Keep documents in English.

When updating requirements or design, make the system boundary explicit:

- user-space FUSE, no kernel changes
- public C ABI, private IPC
- generic Promise filesystem, not a clipboard application
- materialize as a core filesystem operation

