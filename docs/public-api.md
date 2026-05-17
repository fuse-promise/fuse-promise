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

The future public header should live at:

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
typedef enum fp_status {
    FP_OK = 0,
    FP_ERR_INVALID_ARGUMENT,
    FP_ERR_UNAVAILABLE,
    FP_ERR_PERMISSION,
    FP_ERR_NOT_FOUND,
    FP_ERR_ALREADY_EXISTS,
    FP_ERR_PROVIDER_GONE,
    FP_ERR_IO,
    FP_ERR_TIMEOUT,
    FP_ERR_CANCELLED,
    FP_ERR_VERSION_MISMATCH
} fp_status_t;
```

## API Sketch

This is a design sketch, not a frozen ABI.

```c
typedef struct fp_context_options {
    uint32_t api_version;
    const char *runtime_dir;
} fp_context_options_t;

fp_status_t fp_context_open(
    const fp_context_options_t *options,
    fp_context_t **out_context);

void fp_context_close(fp_context_t *context);
```

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
    void *buffer;
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

fp_status_t fp_provider_register(
    fp_context_t *context,
    const fp_provider_ops_t *ops,
    void *user_data,
    fp_provider_t **out_provider);

void fp_provider_unregister(fp_provider_t *provider);
```

Promise creation:

```c
typedef struct fp_node_attr {
    uint32_t mode;
    uint64_t size;
    int64_t mtime_nsec;
} fp_node_attr_t;

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

Materialize:

```c
typedef enum fp_conflict_policy {
    FP_CONFLICT_FAIL = 0,
    FP_CONFLICT_OVERWRITE,
    FP_CONFLICT_RENAME
} fp_conflict_policy_t;

typedef struct fp_materialize_options {
    uint32_t struct_size;
    fp_conflict_policy_t conflict_policy;
} fp_materialize_options_t;

fp_status_t fp_materialize(
    fp_context_t *context,
    const char *promise_path,
    const char *target_dir,
    const fp_materialize_options_t *options);
```

## Usage Pattern

```c
fp_context_t *ctx = NULL;
fp_context_open(NULL, &ctx);

fp_provider_t *provider = NULL;
fp_provider_register(ctx, &ops, user_data, &provider);

fp_promise_builder_t *builder = NULL;
fp_promise_builder_new(ctx, provider, &builder);

fp_promise_add_dir(builder, "photos", &dir_attr, "remote-dir-1");
fp_promise_add_file(builder, "photos/a.jpg", &file_attr, "remote-file-1");

char path[4096];
fp_promise_commit(builder, path, sizeof(path));

/* The provider process must remain alive while the promise can be read. */
```

## Compatibility Rules

- Public structs must include `struct_size` when future extension is expected.
- New enum values may be added.
- Existing enum values must not be renumbered.
- Public functions must keep stable symbol names after the first ABI release.
- Internal IPC messages must not be documented as public API.

