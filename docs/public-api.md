# Public API

## ABI Principle

The public API is a versioned C ABI exported by `libfusepromise.so`.

Applications must include:

```c
#include <fuse-promise/fuse-promise.h>
```

Applications must link through:

```sh
pkg-config --cflags --libs fuse-promise
```

The API hides all daemon communication. Internal IPC is not stable and must not be used by applications.

## Header Ownership

The public header lives at:

```text
include/fuse-promise/fuse-promise.h
```

Installed path:

```text
/usr/include/fuse-promise/fuse-promise.h
```

## Naming

Public symbols use the `fp_` prefix.

Opaque handles:

```c
typedef struct fp_context fp_context_t;
typedef struct fp_provider fp_provider_t;
typedef struct fp_promise_builder fp_promise_builder_t;
typedef struct fp_materialize_job fp_materialize_job_t;
```

Status values:

```c
typedef uint32_t fp_status_t;

#define FP_OK ((fp_status_t)0u)
#define FP_ERR_INVALID_ARGUMENT ((fp_status_t)1u)
#define FP_ERR_UNAVAILABLE ((fp_status_t)2u)
#define FP_ERR_PERMISSION ((fp_status_t)3u)
#define FP_ERR_NOT_FOUND ((fp_status_t)4u)
#define FP_ERR_ALREADY_EXISTS ((fp_status_t)5u)
#define FP_ERR_PROVIDER_GONE ((fp_status_t)6u)
#define FP_ERR_IO ((fp_status_t)7u)
#define FP_ERR_TIMEOUT ((fp_status_t)8u)
#define FP_ERR_CANCELLED ((fp_status_t)9u)
#define FP_ERR_VERSION_MISMATCH ((fp_status_t)10u)

const char *fp_status_string(fp_status_t status);
```

## API Sketch

This API is an initial implementation surface, not a frozen stable ABI.

```c
typedef struct fp_context_options {
    uint32_t struct_size;
    uint32_t api_version;
    const char *runtime_dir;
} fp_context_options_t;

#define FP_CONTEXT_OPTIONS_INIT \
    { sizeof(fp_context_options_t), FP_API_VERSION, NULL }

fp_status_t fp_context_open(
    const fp_context_options_t *options,
    fp_context_t **out_context);

void fp_context_close(fp_context_t *context);
```

Passing `NULL` for options means the current ABI default options. Non-NULL
options must set `struct_size` and `api_version`.

Provider registration:

```c
typedef struct fp_read_request {
    const char *promise_id;
    const char *node_id;
    const char *relative_path;
    uint64_t offset;
    size_t length;
} fp_read_request_t;

typedef struct fp_read_response {
    uint8_t *buffer;
    size_t buffer_len;
    size_t bytes_written;
} fp_read_response_t;

typedef fp_status_t (*fp_provider_read_fn)(
    const fp_read_request_t *request,
    fp_read_response_t *response,
    void *user_data);

typedef struct fp_provider_ops {
    uint32_t struct_size;
    fp_provider_read_fn read;
} fp_provider_ops_t;

#define FP_PROVIDER_OPS_INIT(read_fn) \
    { sizeof(fp_provider_ops_t), (read_fn) }

fp_status_t fp_provider_register(
    fp_context_t *context,
    const fp_provider_ops_t *ops,
    void *user_data,
    fp_provider_t **out_provider);

void fp_provider_unregister(fp_provider_t *provider);
```

The current implementation registers providers with `fuse-promised` through
private daemon IPC. If the daemon is unavailable, provider registration returns
`FP_ERR_UNAVAILABLE`. Provider read callbacks are validated by the public ABI
but are not routed from FUSE reads yet.

Promise creation:

```c
typedef struct fp_node_attr {
    uint32_t struct_size;
    uint32_t mode;
    uint64_t size;
    int64_t mtime_nsec;
} fp_node_attr_t;

#define FP_NODE_ATTR_INIT \
    { sizeof(fp_node_attr_t), 0u, 0u, 0 }

fp_status_t fp_promise_builder_new(
    fp_context_t *context,
    fp_provider_t *provider,
    fp_promise_builder_t **out_builder);

fp_status_t fp_promise_add_dir(
    fp_promise_builder_t *builder,
    const char *relative_path,
    const fp_node_attr_t *attr,
    const char *provider_node_id);

fp_status_t fp_promise_add_file(
    fp_promise_builder_t *builder,
    const char *relative_path,
    const fp_node_attr_t *attr,
    const char *provider_node_id);

fp_status_t fp_promise_commit(
    fp_promise_builder_t *builder,
    char *out_path,
    size_t out_path_len);

void fp_promise_builder_free(fp_promise_builder_t *builder);
```

`mode` contains Unix permission bits only, such as `0644` or `0755`. The
runtime derives the file type from whether the caller adds a file or directory.
Directories must use size `0`.

Materialize:

```c
typedef uint32_t fp_conflict_policy_t;

#define FP_CONFLICT_FAIL ((fp_conflict_policy_t)0u)
#define FP_CONFLICT_OVERWRITE ((fp_conflict_policy_t)1u)
#define FP_CONFLICT_RENAME ((fp_conflict_policy_t)2u)

typedef struct fp_materialize_options {
    uint32_t struct_size;
    fp_conflict_policy_t conflict_policy;
} fp_materialize_options_t;

#define FP_MATERIALIZE_OPTIONS_INIT \
    { sizeof(fp_materialize_options_t), FP_CONFLICT_FAIL }

fp_status_t fp_materialize(
    fp_context_t *context,
    const char *promise_path,
    const char *target_dir,
    const fp_materialize_options_t *options);
```

The current implementation returns `FP_ERR_UNAVAILABLE` from
`fp_promise_commit()` and `fp_materialize()` until the daemon IPC, FUSE adapter,
and materialize engine are implemented. The runtime crate can validate and hold
metadata in process, but the public library must not claim a visible FUSE path
until the daemon owns that namespace.

## String and Buffer Rules

The initial ABI accepts NUL-terminated UTF-8 strings for promise paths,
provider node identifiers, and runtime directory overrides. This keeps the
developer preview small. A future byte-path API may be added before the first
stable ABI if full Linux non-UTF-8 filename support is required.

Provider read callbacks receive a runtime-owned writable buffer:

- `response->buffer` points to storage owned by `libfusepromise.so`.
- `response->buffer_len` is the maximum number of bytes the provider may write.
- The provider sets `response->bytes_written` to the number of bytes produced.
- The provider must not retain the buffer pointer after the callback returns.

## Usage Pattern

```c
fp_context_options_t options = FP_CONTEXT_OPTIONS_INIT;
fp_context_t *ctx = NULL;
fp_context_open(&options, &ctx);

fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_cb);
fp_provider_t *provider = NULL;
fp_provider_register(ctx, &ops, user_data, &provider);

fp_promise_builder_t *builder = NULL;
fp_promise_builder_new(ctx, provider, &builder);

fp_node_attr_t dir_attr = FP_NODE_ATTR_INIT;
dir_attr.mode = 0755;

fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
file_attr.mode = 0644;
file_attr.size = 1234;

fp_promise_add_dir(builder, "photos", &dir_attr, "remote-dir-1");
fp_promise_add_file(builder, "photos/a.jpg", &file_attr, "remote-file-1");

char path[4096];
fp_status_t status = fp_promise_commit(builder, path, sizeof(path));
/* The current skeleton returns FP_ERR_UNAVAILABLE until daemon IPC exists. */
(void)status;

/* The provider process must remain alive while the promise can be read. */
```

## Compatibility Rules

- Public structs must include `struct_size` unless they are explicitly frozen.
- New status and policy values may be added.
- Existing status and policy values must not be renumbered.
- Public functions must keep stable symbol names after the first ABI release.
- Internal IPC messages must not be documented as public API.
