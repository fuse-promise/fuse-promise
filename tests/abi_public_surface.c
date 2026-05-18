#define _POSIX_C_SOURCE 200809L

#include <fuse-promise/fuse-promise.h>

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                     \
            fprintf(stderr, "check failed: %s:%d: %s\n", __FILE__, __LINE__,    \
                    #condition);                                               \
            return 1;                                                          \
        }                                                                      \
    } while (0)

_Static_assert(FP_API_VERSION == 1u, "FP_API_VERSION changed");
_Static_assert(sizeof(fp_status_t) == sizeof(uint32_t),
               "fp_status_t must be uint32_t");
_Static_assert(FP_OK == 0u, "FP_OK changed");
_Static_assert(FP_ERR_INVALID_ARGUMENT == 1u,
               "FP_ERR_INVALID_ARGUMENT changed");
_Static_assert(FP_ERR_UNAVAILABLE == 2u, "FP_ERR_UNAVAILABLE changed");
_Static_assert(FP_ERR_PERMISSION == 3u, "FP_ERR_PERMISSION changed");
_Static_assert(FP_ERR_NOT_FOUND == 4u, "FP_ERR_NOT_FOUND changed");
_Static_assert(FP_ERR_ALREADY_EXISTS == 5u, "FP_ERR_ALREADY_EXISTS changed");
_Static_assert(FP_ERR_PROVIDER_GONE == 6u, "FP_ERR_PROVIDER_GONE changed");
_Static_assert(FP_ERR_IO == 7u, "FP_ERR_IO changed");
_Static_assert(FP_ERR_TIMEOUT == 8u, "FP_ERR_TIMEOUT changed");
_Static_assert(FP_ERR_CANCELLED == 9u, "FP_ERR_CANCELLED changed");
_Static_assert(FP_ERR_VERSION_MISMATCH == 10u,
               "FP_ERR_VERSION_MISMATCH changed");

_Static_assert(FP_CONFLICT_FAIL == 0u, "FP_CONFLICT_FAIL changed");
_Static_assert(FP_CONFLICT_OVERWRITE == 1u,
               "FP_CONFLICT_OVERWRITE changed");
_Static_assert(FP_CONFLICT_RENAME == 2u, "FP_CONFLICT_RENAME changed");

_Static_assert(sizeof(fp_context_options_t) == 16u,
               "fp_context_options_t size changed");
_Static_assert(_Alignof(fp_context_options_t) == 8u,
               "fp_context_options_t alignment changed");
_Static_assert(offsetof(fp_context_options_t, struct_size) == 0u,
               "fp_context_options_t.struct_size offset changed");
_Static_assert(offsetof(fp_context_options_t, api_version) == 4u,
               "fp_context_options_t.api_version offset changed");
_Static_assert(offsetof(fp_context_options_t, runtime_dir) == 8u,
               "fp_context_options_t.runtime_dir offset changed");

_Static_assert(sizeof(fp_read_request_t) == 40u,
               "fp_read_request_t size changed");
_Static_assert(_Alignof(fp_read_request_t) == 8u,
               "fp_read_request_t alignment changed");
_Static_assert(offsetof(fp_read_request_t, promise_id) == 0u,
               "fp_read_request_t.promise_id offset changed");
_Static_assert(offsetof(fp_read_request_t, node_id) == 8u,
               "fp_read_request_t.node_id offset changed");
_Static_assert(offsetof(fp_read_request_t, relative_path) == 16u,
               "fp_read_request_t.relative_path offset changed");
_Static_assert(offsetof(fp_read_request_t, offset) == 24u,
               "fp_read_request_t.offset offset changed");
_Static_assert(offsetof(fp_read_request_t, length) == 32u,
               "fp_read_request_t.length offset changed");

_Static_assert(sizeof(fp_read_response_t) == 24u,
               "fp_read_response_t size changed");
_Static_assert(_Alignof(fp_read_response_t) == 8u,
               "fp_read_response_t alignment changed");
_Static_assert(offsetof(fp_read_response_t, buffer) == 0u,
               "fp_read_response_t.buffer offset changed");
_Static_assert(offsetof(fp_read_response_t, buffer_len) == 8u,
               "fp_read_response_t.buffer_len offset changed");
_Static_assert(offsetof(fp_read_response_t, bytes_written) == 16u,
               "fp_read_response_t.bytes_written offset changed");

_Static_assert(sizeof(fp_provider_ops_t) == 16u,
               "fp_provider_ops_t size changed");
_Static_assert(_Alignof(fp_provider_ops_t) == 8u,
               "fp_provider_ops_t alignment changed");
_Static_assert(offsetof(fp_provider_ops_t, struct_size) == 0u,
               "fp_provider_ops_t.struct_size offset changed");
_Static_assert(offsetof(fp_provider_ops_t, read) == 8u,
               "fp_provider_ops_t.read offset changed");

_Static_assert(sizeof(fp_node_attr_t) == 24u, "fp_node_attr_t size changed");
_Static_assert(_Alignof(fp_node_attr_t) == 8u,
               "fp_node_attr_t alignment changed");
_Static_assert(offsetof(fp_node_attr_t, struct_size) == 0u,
               "fp_node_attr_t.struct_size offset changed");
_Static_assert(offsetof(fp_node_attr_t, mode) == 4u,
               "fp_node_attr_t.mode offset changed");
_Static_assert(offsetof(fp_node_attr_t, size) == 8u,
               "fp_node_attr_t.size offset changed");
_Static_assert(offsetof(fp_node_attr_t, mtime_nsec) == 16u,
               "fp_node_attr_t.mtime_nsec offset changed");

_Static_assert(sizeof(fp_materialize_options_t) == 8u,
               "fp_materialize_options_t size changed");
_Static_assert(_Alignof(fp_materialize_options_t) == 4u,
               "fp_materialize_options_t alignment changed");
_Static_assert(offsetof(fp_materialize_options_t, struct_size) == 0u,
               "fp_materialize_options_t.struct_size offset changed");
_Static_assert(offsetof(fp_materialize_options_t, conflict_policy) == 4u,
               "fp_materialize_options_t.conflict_policy offset changed");

static int expect_status_string(fp_status_t status, const char *expected) {
    const char *actual = fp_status_string(status);
    if (actual == NULL || strcmp(actual, expected) != 0) {
        fprintf(stderr, "status string mismatch for %u: got %s expected %s\n",
                status, actual == NULL ? "(null)" : actual, expected);
        return 1;
    }
    return 0;
}

static fp_status_t example_read(const fp_read_request_t *request,
                                fp_read_response_t *response,
                                void *user_data) {
    (void)request;
    (void)response;
    (void)user_data;
    return FP_ERR_NOT_FOUND;
}

int main(void) {
    CHECK(expect_status_string(FP_OK, "ok") == 0);
    CHECK(expect_status_string(FP_ERR_INVALID_ARGUMENT, "invalid argument") ==
          0);
    CHECK(expect_status_string(FP_ERR_UNAVAILABLE, "unavailable") == 0);
    CHECK(expect_status_string(FP_ERR_PERMISSION, "permission denied") == 0);
    CHECK(expect_status_string(FP_ERR_NOT_FOUND, "not found") == 0);
    CHECK(expect_status_string(FP_ERR_ALREADY_EXISTS, "already exists") == 0);
    CHECK(expect_status_string(FP_ERR_PROVIDER_GONE, "provider gone") == 0);
    CHECK(expect_status_string(FP_ERR_IO, "io error") == 0);
    CHECK(expect_status_string(FP_ERR_TIMEOUT, "timeout") == 0);
    CHECK(expect_status_string(FP_ERR_CANCELLED, "cancelled") == 0);
    CHECK(expect_status_string(FP_ERR_VERSION_MISMATCH, "version mismatch") ==
          0);
    CHECK(expect_status_string(999u, "unknown status") == 0);

    fp_context_options_t context_options = FP_CONTEXT_OPTIONS_INIT;
    CHECK(context_options.struct_size == sizeof(fp_context_options_t));
    CHECK(context_options.api_version == FP_API_VERSION);
    CHECK(context_options.runtime_dir == NULL);

    fp_provider_ops_t provider_ops = FP_PROVIDER_OPS_INIT(example_read);
    CHECK(provider_ops.struct_size == sizeof(fp_provider_ops_t));
    CHECK(provider_ops.read == example_read);

    fp_node_attr_t node_attr = FP_NODE_ATTR_INIT;
    CHECK(node_attr.struct_size == sizeof(fp_node_attr_t));
    CHECK(node_attr.mode == 0u);
    CHECK(node_attr.size == 0u);
    CHECK(node_attr.mtime_nsec == 0);

    fp_materialize_options_t materialize_options =
        FP_MATERIALIZE_OPTIONS_INIT;
    CHECK(materialize_options.struct_size == sizeof(fp_materialize_options_t));
    CHECK(materialize_options.conflict_policy == FP_CONFLICT_FAIL);

    fp_context_t *context = NULL;
    CHECK(fp_context_open(&context_options, NULL) == FP_ERR_INVALID_ARGUMENT);

    fp_context_options_t bad_size = context_options;
    bad_size.struct_size = 0;
    CHECK(fp_context_open(&bad_size, &context) == FP_ERR_INVALID_ARGUMENT);
    CHECK(context == NULL);

    fp_context_options_t bad_version = context_options;
    bad_version.api_version = FP_API_VERSION + 1u;
    CHECK(fp_context_open(&bad_version, &context) == FP_ERR_VERSION_MISMATCH);
    CHECK(context == NULL);

    char template[] = "/tmp/fuse-promise-abi-XXXXXX";
    char *runtime_dir = mkdtemp(template);
    CHECK(runtime_dir != NULL);
    CHECK(chmod(runtime_dir, 0700) == 0);
    context_options.runtime_dir = runtime_dir;
    CHECK(fp_context_open(&context_options, &context) == FP_OK);
    CHECK(context != NULL);

    CHECK(fp_provider_register(NULL, NULL, NULL, NULL) ==
          FP_ERR_INVALID_ARGUMENT);
    CHECK(fp_promise_builder_new(NULL, NULL, NULL) == FP_ERR_INVALID_ARGUMENT);
    CHECK(fp_promise_add_dir(NULL, "docs", &node_attr, "remote-dir") ==
          FP_ERR_INVALID_ARGUMENT);
    CHECK(fp_promise_add_file(NULL, "docs/readme.txt", &node_attr,
                              "remote-file") == FP_ERR_INVALID_ARGUMENT);
    CHECK(fp_promise_commit(NULL, NULL, 0) == FP_ERR_INVALID_ARGUMENT);
    CHECK(fp_materialize(NULL, "/missing", runtime_dir, NULL) ==
          FP_ERR_INVALID_ARGUMENT);

    fp_provider_unregister(NULL);
    fp_promise_builder_free(NULL);
    fp_context_close(NULL);
    fp_context_close(context);
    CHECK(rmdir(runtime_dir) == 0);

    return 0;
}
