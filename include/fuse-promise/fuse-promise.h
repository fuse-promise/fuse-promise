#ifndef FUSE_PROMISE_FUSE_PROMISE_H
#define FUSE_PROMISE_FUSE_PROMISE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FP_API_VERSION 1u

typedef struct fp_context fp_context_t;
typedef struct fp_provider fp_provider_t;
typedef struct fp_promise_builder fp_promise_builder_t;
typedef struct fp_materialize_job fp_materialize_job_t;

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

typedef struct fp_context_options {
    uint32_t struct_size;
    uint32_t api_version;
    const char *runtime_dir;
} fp_context_options_t;

#define FP_CONTEXT_OPTIONS_INIT \
    { sizeof(fp_context_options_t), FP_API_VERSION, NULL }

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

typedef struct fp_node_attr {
    uint32_t struct_size;
    uint32_t mode;
    uint64_t size;
    int64_t mtime_nsec;
} fp_node_attr_t;

#define FP_NODE_ATTR_INIT \
    { sizeof(fp_node_attr_t), 0u, 0u, 0 }

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

const char *fp_status_string(fp_status_t status);

fp_status_t fp_context_open(
    const fp_context_options_t *options,
    fp_context_t **out_context);

void fp_context_close(fp_context_t *context);

fp_status_t fp_provider_register(
    fp_context_t *context,
    const fp_provider_ops_t *ops,
    void *user_data,
    fp_provider_t **out_provider);

void fp_provider_unregister(fp_provider_t *provider);

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

fp_status_t fp_materialize(
    fp_context_t *context,
    const char *promise_path,
    const char *target_dir,
    const fp_materialize_options_t *options);

#ifdef __cplusplus
}
#endif

#endif
