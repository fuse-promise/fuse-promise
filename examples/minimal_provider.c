#include <fuse-promise/fuse-promise.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char kData[] = "hello from fuse-promise example\n";

static fp_status_t read_file(const fp_read_request_t *request,
                             fp_read_response_t *response,
                             void *user_data) {
    (void)user_data;
    if (request == NULL || response == NULL || response->buffer == NULL) {
        return FP_ERR_INVALID_ARGUMENT;
    }
    if (strcmp(request->node_id, "example-file") != 0) {
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
    if (argc != 2) {
        fprintf(stderr, "usage: minimal_provider <runtime-dir>\n");
        return 2;
    }

    fp_context_options_t options = FP_CONTEXT_OPTIONS_INIT;
    options.runtime_dir = argv[1];

    fp_context_t *context = NULL;
    fp_status_t status = fp_context_open(&options, &context);
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

    fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
    file_attr.mode = 0644;
    file_attr.size = strlen(kData);
    status = fp_promise_add_file(builder, "hello.txt", &file_attr,
                                 "example-file");
    if (status != FP_OK) {
        fail("fp_promise_add_file", status);
    }

    char visible_path[4096];
    status = fp_promise_commit(builder, visible_path, sizeof(visible_path));
    if (status != FP_OK) {
        fail("fp_promise_commit", status);
    }

    printf("%s\n", visible_path);

    fp_promise_builder_free(builder);
    fp_provider_unregister(provider);
    fp_context_close(context);
    return 0;
}
