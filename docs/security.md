# Security Model

`fuse-promise` is a Linux user-session filesystem component. Its security
model is scoped to one local Unix user session and does not attempt to provide a
cross-user, network, cloud-provider, or desktop-application authorization
layer.

## Session Boundary

The default runtime namespace is derived from `XDG_RUNTIME_DIR`:

```text
$XDG_RUNTIME_DIR/fuse-promise/       FUSE mount
$XDG_RUNTIME_DIR/fuse-promise.sock   private daemon control socket
```

The runtime directory must be absolute, owned by the current user, a directory,
and inaccessible to group or other users. `fuse-promise` fails explicitly when
the directory is missing, unsafe, foreign-owned, or too broadly accessible.

The FUSE mount is user-session scoped. The daemon does not create a global
shared mount and does not install a kernel component.

Processes running under the same Unix UID are treated as part of the same local
session trust boundary. Fine-grained application authorization between same-UID
processes is out of scope for the core component.

## Public and Private Interfaces

External consumers use only the public C ABI:

```text
fuse-promise/fuse-promise.h
libfusepromise.so
pkg-config --cflags --libs fuse-promise
```

The Unix socket protocol is private implementation detail between
`libfusepromise.so`, `fpctl`, and `fuse-promised`. Applications must not depend
on the socket path, message types, or framing format.

Before connecting, IPC clients verify that the control path is a Unix socket
owned by the current user. Before binding, the daemon refuses to remove a stale
control socket unless that path is a Unix socket owned by the current user. The
daemon also checks Unix peer credentials on accepted connections and rejects
peers whose UID differs from the daemon UID.

## Provider and Promise Ownership

Committed Promise trees are owned by `fuse-promised`. Provider processes receive
opaque provider ids and private owner tokens through the private IPC path.
Provider unregister and Promise commit mutations require the matching owner
token, and provider read responses are accepted only from the connection that
registered the provider.

Provider disconnect marks non-materialized and non-cache-satisfied promises
unavailable. Complete materialized files, and complete cached ranges in
read-through mode, may continue to satisfy reads without the provider.

## Paths and Materialize Targets

Promise metadata paths are normalized relative paths. Absolute paths, parent
traversal, NUL-containing paths, duplicate nodes, missing parents, and invalid
node metadata are rejected before daemon state mutates.

Materialize uses the same provider read path as lazy filesystem reads. The
current fail-on-conflict policy rejects symlink target directories and existing
targets, including existing symlink targets, during preflight. Created target
identity is tracked during cleanup so later failures do not remove unrelated
files.

## Non-Goals

This repository does not provide:

- Cross-user sharing.
- Network authentication.
- Cloud-provider credentials.
- P2P transport security.
- Desktop-environment policy.
- Application-specific authorization.

Those concerns belong in external applications that consume the public C ABI.
