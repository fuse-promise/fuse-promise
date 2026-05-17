# Promise Model

## Terms

### Promise

A Promise is a committed virtual filesystem tree whose node metadata is available before local file content exists.

### Promised File

A promised file is a regular-looking file node. Its size, mode, and timestamps are declared, but its bytes are supplied lazily by a provider.

### Promised Directory

A promised directory is a directory node whose children are declared by metadata. Enumerating the directory must not require reading file bytes.

### Provider

A provider is the owner of promised content. It responds to runtime read requests for nodes it registered.

### Lazy Read

A lazy read occurs when an ordinary filesystem read reaches a promised file whose content is not already materialized or cached. The runtime asks the provider for the requested byte range.

### Materialize

Materialize is the operation that writes a promised file or directory subtree into real local filesystem storage.

### Materialized Node

A materialized node is a promised node whose full content exists at a real local path.

## Node Identity

Each node should have:

- A stable runtime node id.
- A provider-owned opaque id.
- A normalized relative path.
- A type.

The runtime node id is internal. The provider-owned id allows the provider to route read requests without relying only on paths.

## Metadata

Minimum file metadata:

```text
type
size
mode
mtime
provider_node_id
```

Recommended metadata:

```text
ctime
uid
gid
content_hash
mime_type
capabilities
```

The first stable release should keep the required metadata small and add optional fields only when they have clear behavior.

## Read Semantics

The runtime should prefer offset-based reads:

```text
read(provider_node_id, offset, length) -> bytes
```

Rules:

- A read past end-of-file returns zero bytes.
- A short read is allowed.
- The runtime may retry provider reads according to policy.
- The provider must not return bytes outside the requested file.
- The provider must not change file size during an active read unless the node was explicitly declared mutable.

## Directory Semantics

Directory children are declared metadata. A directory listing should be complete for the committed snapshot unless the tree was declared dynamic.

The MVP should implement snapshot directories only.

## Mutation Semantics

The initial stable model should be read-only.

Write support can be added later with explicit capabilities:

- Append.
- Overwrite.
- Create.
- Delete.
- Rename.
- Provider-side commit.

Read-only first keeps the ABI smaller and makes materialize behavior easier to define.

## Materialize Semantics

Materialize copies promised content into real local storage.

For a file:

1. Create or open the target file according to conflict policy.
2. Read provider content in chunks.
3. Write chunks to the target file.
4. Apply mode and timestamps.
5. Mark the node materialized if the operation succeeds.

For a directory:

1. Create the target directory.
2. Recursively materialize children.
3. Apply directory metadata.
4. Report partial failure with a structured result.

## Snapshot Semantics

A committed Promise tree is a metadata snapshot by default.

This means:

- Paths are known at commit time.
- File sizes are stable unless declared otherwise.
- Directory entries are stable unless declared dynamic.
- Lazy reads supply content for that snapshot.

Dynamic trees may be designed later, but they should be explicit.

