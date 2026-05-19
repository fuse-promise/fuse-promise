# Public API

This page describes how an application uses `fuse-promise` after the package is
installed.

The public API is the C ABI exported by `libfusepromise.so`. Applications do not
talk to daemon IPC directly.

## Build Against the Library

Include the public header:

```c
#include <fuse-promise/fuse-promise.h>
```

Compile and link with `pkg-config`:

```sh
cc provider.c $(pkg-config --cflags --libs fuse-promise) -o provider
```

Installed files used by application developers:

```text
/usr/include/fuse-promise/fuse-promise.h
/usr/lib/libfusepromise.so
/usr/lib/pkgconfig/fuse-promise.pc
```

The daemon must be running before a provider can publish a visible Promise tree:

```sh
systemctl --user start fuse-promised
```

For manual testing:

```sh
fuse-promised --foreground
```

## Basic Flow

A provider usually does this:

1. Open a `fp_context_t`.
2. Register a provider read callback.
3. Create a Promise builder.
4. Add directories and files with attributes.
5. Commit the tree and receive the visible path.
6. Keep the provider process alive while files may be read.
7. Optionally materialize the promised path into local storage.
8. Unregister the provider and close the context.

Mounted writes are not a public callback today. Writing promised content into
local storage is done through `fp_materialize()` or `fpctl materialize`.

## Status Values

Most functions return `fp_status_t`.

| Status | Meaning for callers |
|---|---|
| `FP_OK` | The operation completed. |
| `FP_ERR_INVALID_ARGUMENT` | A pointer, struct size, enum value, path, or buffer is invalid. |
| `FP_ERR_UNAVAILABLE` | The daemon, mount, runtime directory, or requested mode is unavailable. |
| `FP_ERR_PERMISSION` | The operation is not allowed for this user or provider. |
| `FP_ERR_NOT_FOUND` | The Promise, node, path, or provider object was not found. |
| `FP_ERR_ALREADY_EXISTS` | The target already exists and the conflict policy does not allow it. |
| `FP_ERR_PROVIDER_GONE` | The provider disconnected before bytes could be supplied. |
| `FP_ERR_IO` | An underlying I/O operation failed. |
| `FP_ERR_TIMEOUT` | The daemon or provider did not answer in time. |
| `FP_ERR_CANCELLED` | A cancellable operation was cancelled. |
| `FP_ERR_VERSION_MISMATCH` | The caller and runtime use incompatible API versions. |

Use `fp_status_string()` when printing errors.

## `fp_status_string`

```c
const char *fp_status_string(fp_status_t status);
```

Returns a stable English string for a status value.

Use it for logs, diagnostics, and command-line errors:

```c
fprintf(stderr, "fp_context_open: %s\n", fp_status_string(status));
```

## `fp_context_open`

```c
fp_status_t fp_context_open(
    const fp_context_options_t *options,
    fp_context_t **out_context);
```

Opens a client context.

Use `FP_CONTEXT_OPTIONS_INIT` so the struct size and API version are set:

```c
fp_context_options_t options = FP_CONTEXT_OPTIONS_INIT;
options.runtime_dir = runtime_dir;

fp_context_t *context = NULL;
fp_status_t status = fp_context_open(&options, &context);
```

`runtime_dir` should match the runtime directory used by `fuse-promised`. For a
normal user session this is usually `XDG_RUNTIME_DIR`. Passing `NULL` options
uses the default runtime directory.

## `fp_context_close`

```c
void fp_context_close(fp_context_t *context);
```

Closes a context returned by `fp_context_open()`.

Unregister providers and free active builders before closing the context.
Passing `NULL` is allowed.

## `fp_provider_read_fn`

```c
typedef fp_status_t (*fp_provider_read_fn)(
    const fp_read_request_t *request,
    fp_read_response_t *response,
    void *user_data);
```

This callback supplies bytes for promised files.

The runtime calls it when a process reads a file through the FUSE mount or when
materialize needs the file contents.

Request fields:

| Field | Meaning |
|---|---|
| `promise_id` | Runtime Promise identifier. |
| `node_id` | Provider node identifier passed to `fp_promise_add_file()`. |
| `relative_path` | Path of the file inside the Promise tree. |
| `offset` | First byte requested. |
| `length` | Maximum number of bytes requested. |

Response fields:

| Field | Rule |
|---|---|
| `buffer` | Writable buffer owned by the runtime. |
| `buffer_len` | Maximum bytes the callback may write. |
| `bytes_written` | Number of bytes actually written. |

The callback must not retain `response->buffer` after it returns.

Return `FP_OK` for a successful read, including end-of-file with
`bytes_written = 0`. Return `FP_ERR_NOT_FOUND` if the requested provider node is
unknown.

## `fp_provider_register`

```c
fp_status_t fp_provider_register(
    fp_context_t *context,
    const fp_provider_ops_t *ops,
    void *user_data,
    fp_provider_t **out_provider);
```

Registers a provider with the daemon.

The provider must include a read callback:

```c
fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_file);

fp_provider_t *provider = NULL;
fp_status_t status = fp_provider_register(context, &ops, user_data, &provider);
```

`user_data` is passed back to the read callback. The provider process must stay
alive while any promised files can still be read.

## `fp_provider_unregister`

```c
void fp_provider_unregister(fp_provider_t *provider);
```

Unregisters a provider returned by `fp_provider_register()`.

After unregistering, reads for non-materialized content owned by that provider
can fail with `FP_ERR_PROVIDER_GONE` through the C API or an I/O error through
the mounted filesystem. Passing `NULL` is allowed.

## `fp_promise_builder_new`

```c
fp_status_t fp_promise_builder_new(
    fp_context_t *context,
    fp_provider_t *provider,
    fp_promise_builder_t **out_builder);
```

Creates a builder for one Promise tree.

All files added to the builder are owned by the provider passed here.

```c
fp_promise_builder_t *builder = NULL;
fp_status_t status = fp_promise_builder_new(context, provider, &builder);
```

## `fp_promise_add_dir`

```c
fp_status_t fp_promise_add_dir(
    fp_promise_builder_t *builder,
    const char *relative_path,
    const fp_node_attr_t *attr,
    const char *provider_node_id);
```

Adds a directory to the Promise tree.

Use a relative path without a leading slash:

```c
fp_node_attr_t dir_attr = FP_NODE_ATTR_INIT;
dir_attr.mode = 0755;
dir_attr.mtime_nsec = 1700000000000000000LL;

fp_promise_add_dir(builder, "docs", &dir_attr, "docs-dir");
```

Directory attributes:

| Field | Rule |
|---|---|
| `mode` | Permission bits, for example `0755`. |
| `size` | Must be `0` for directories. |
| `mtime_nsec` | Unix epoch timestamp in nanoseconds. |

`provider_node_id` is your stable identifier for this directory.

## `fp_promise_add_file`

```c
fp_status_t fp_promise_add_file(
    fp_promise_builder_t *builder,
    const char *relative_path,
    const fp_node_attr_t *attr,
    const char *provider_node_id);
```

Adds a file to the Promise tree.

```c
fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
file_attr.mode = 0644;
file_attr.size = file_size;
file_attr.mtime_nsec = 1700000000000000000LL;

fp_promise_add_file(builder, "docs/hello.txt", &file_attr, "hello-file");
```

File attributes:

| Field | Rule |
|---|---|
| `mode` | Permission bits, for example `0644`. |
| `size` | Visible file size reported by `stat`. |
| `mtime_nsec` | Unix epoch timestamp in nanoseconds. |

`provider_node_id` is returned later in `fp_read_request_t.node_id`, so your
callback can find the file contents.

## `fp_promise_commit`

```c
fp_status_t fp_promise_commit(
    fp_promise_builder_t *builder,
    char *out_path,
    size_t out_path_len);
```

Publishes the Promise tree to the mounted filesystem.

On success, `out_path` receives the visible root path:

```c
char visible_path[4096];
fp_status_t status =
    fp_promise_commit(builder, visible_path, sizeof(visible_path));
```

After commit, normal Linux tools can inspect the tree:

```sh
find "$visible_path" -maxdepth 2 -type f -print
cat "$visible_path/docs/hello.txt"
```

The provider still needs to stay alive so reads can be served.

## `fp_promise_builder_free`

```c
void fp_promise_builder_free(fp_promise_builder_t *builder);
```

Frees a builder.

Call this after `fp_promise_commit()` succeeds, or during error cleanup if a
builder is no longer needed. Passing `NULL` is allowed.

## `fp_materialize`

```c
fp_status_t fp_materialize(
    fp_context_t *context,
    const char *promise_path,
    const char *target_dir,
    const fp_materialize_options_t *options);
```

Writes a promised file or directory subtree into local storage.

`promise_path` is a visible path under the mounted Promise filesystem.
`target_dir` is an existing local directory where content should be written.

```c
fp_materialize_options_t options = FP_MATERIALIZE_OPTIONS_INIT;
options.conflict_policy = FP_CONFLICT_RENAME;

fp_status_t status =
    fp_materialize(context, visible_path, "/tmp/out", &options);
```

Conflict policies:

| Policy | Behavior |
|---|---|
| `FP_CONFLICT_FAIL` | Fail if the target exists. |
| `FP_CONFLICT_OVERWRITE` | Replace existing target content where allowed. |
| `FP_CONFLICT_RENAME` | Pick a non-existing target name. |

If `options.progress` is set, the callback receives best-effort progress
snapshots. Returning `FP_ERR_CANCELLED` from the progress callback cancels the
operation.

## String and Buffer Rules

Paths and provider node identifiers are NUL-terminated UTF-8 strings.

Relative paths passed to `fp_promise_add_dir()` and `fp_promise_add_file()`:

- Must not start with `/`.
- Must not contain `..` components.
- Should use `/` as the separator.

Read callbacks must write at most `response->buffer_len` bytes and set
`response->bytes_written`.

## Complete Example Usage

This example publishes `docs/hello.txt`, serves reads from memory, and can
materialize the Promise tree when a target directory is passed as the second
argument.

The complete maintained example is
[`examples/minimal_provider.c`](https://github.com/fuse-promise/fuse-promise/blob/main/examples/minimal_provider.c).

```c
#include <fuse-promise/fuse-promise.h>

#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const char kData[] = "hello from fuse-promise example\n";
static volatile sig_atomic_t keep_running = 1;

static void stop_provider(int signal_number) {
    (void)signal_number;
    keep_running = 0;
}

static fp_status_t read_file(const fp_read_request_t *request,
                             fp_read_response_t *response,
                             void *user_data) {
    (void)user_data;

    if (strcmp(request->node_id, "hello-file") != 0 ||
        strcmp(request->relative_path, "docs/hello.txt") != 0) {
        return FP_ERR_NOT_FOUND;
    }

    size_t size = strlen(kData);
    if (request->offset >= size) {
        response->bytes_written = 0;
        return FP_OK;
    }

    size_t available = size - (size_t)request->offset;
    size_t count = request->length < available ? request->length : available;
    if (count > response->buffer_len) {
        count = response->buffer_len;
    }

    memcpy(response->buffer, kData + request->offset, count);
    response->bytes_written = count;
    return FP_OK;
}

static void fail(const char *label, fp_status_t status) {
    fprintf(stderr, "%s: %s (%u)\n", label, fp_status_string(status), status);
    exit(1);
}

int main(int argc, char **argv) {
    if (argc < 2 || argc > 3) {
        fprintf(stderr, "usage: provider <runtime-dir> [materialize-target]\n");
        return 2;
    }

    signal(SIGTERM, stop_provider);
    signal(SIGINT, stop_provider);

    fp_context_options_t context_options = FP_CONTEXT_OPTIONS_INIT;
    context_options.runtime_dir = argv[1];

    fp_context_t *context = NULL;
    fp_status_t status = fp_context_open(&context_options, &context);
    if (status != FP_OK) {
        fail("fp_context_open", status);
    }

    fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_file);
    fp_provider_t *provider = NULL;
    status = fp_provider_register(context, &ops, NULL, &provider);
    if (status != FP_OK) {
        fail("fp_provider_register", status);
    }

    fp_promise_builder_t *builder = NULL;
    status = fp_promise_builder_new(context, provider, &builder);
    if (status != FP_OK) {
        fail("fp_promise_builder_new", status);
    }

    fp_node_attr_t dir_attr = FP_NODE_ATTR_INIT;
    dir_attr.mode = 0755;
    dir_attr.mtime_nsec = 1700000000000000000LL;
    status = fp_promise_add_dir(builder, "docs", &dir_attr, "docs-dir");
    if (status != FP_OK) {
        fail("fp_promise_add_dir", status);
    }

    fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
    file_attr.mode = 0644;
    file_attr.size = strlen(kData);
    file_attr.mtime_nsec = 1700000000000000000LL;
    status = fp_promise_add_file(builder, "docs/hello.txt", &file_attr,
                                 "hello-file");
    if (status != FP_OK) {
        fail("fp_promise_add_file", status);
    }

    char visible_path[4096];
    status = fp_promise_commit(builder, visible_path, sizeof(visible_path));
    if (status != FP_OK) {
        fail("fp_promise_commit", status);
    }
    fp_promise_builder_free(builder);

    printf("Promise path: %s\n", visible_path);
    fflush(stdout);

    if (argc == 3) {
        fp_materialize_options_t materialize_options =
            FP_MATERIALIZE_OPTIONS_INIT;
        materialize_options.conflict_policy = FP_CONFLICT_RENAME;

        status = fp_materialize(context, visible_path, argv[2],
                                &materialize_options);
        if (status != FP_OK) {
            fail("fp_materialize", status);
        }
    }

    while (keep_running) {
        sleep(1);
    }

    fp_provider_unregister(provider);
    fp_context_close(context);
    return 0;
}
```

Run it with a foreground daemon:

```sh
runtime=$(mktemp -d)
out=$(mktemp -d)

XDG_RUNTIME_DIR="$runtime" fuse-promised --foreground &
daemon_pid=$!

cc provider.c $(pkg-config --cflags --libs fuse-promise) -o provider
./provider "$runtime" "$out" &
provider_pid=$!

cat "$runtime/fuse-promise/promise-1/docs/hello.txt"
find "$out" -maxdepth 2 -type f -print

kill "$provider_pid" "$daemon_pid"
```
