#include <fuse-promise/fuse-promise.h>

#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct file_data {
    const char *node_id;
    const char *relative_path;
    const char *data;
} file_data_t;

static const file_data_t kFiles[] = {
    {"remote-file-1", "docs/readme.txt", "hello from fuse-promise\n"},
    {"remote-file-2", "docs/guides/setup.txt", "setup guide\n"},
    {"remote-file-3", "pending.txt", "pending data\n"},
};
static volatile sig_atomic_t keep_running = 1;

static const file_data_t *find_file(const char *node_id,
                                    const char *relative_path) {
    size_t count = sizeof(kFiles) / sizeof(kFiles[0]);
    for (size_t index = 0; index < count; index++) {
        if (strcmp(kFiles[index].node_id, node_id) == 0 &&
            strcmp(kFiles[index].relative_path, relative_path) == 0) {
            return &kFiles[index];
        }
    }

    return NULL;
}

static void stop_provider(int signal_number) {
    (void)signal_number;
    keep_running = 0;
}

static fp_status_t read_file(const fp_read_request_t *request,
                             fp_read_response_t *response,
                             void *user_data) {
    FILE *log_file = (FILE *)user_data;
    if (request == NULL || response == NULL || response->buffer == NULL ||
        log_file == NULL) {
        return FP_ERR_INVALID_ARGUMENT;
    }
    const file_data_t *file =
        find_file(request->node_id, request->relative_path);
    if (file == NULL) {
        return FP_ERR_INVALID_ARGUMENT;
    }

    fprintf(log_file, "READ offset=%llu length=%zu\n",
            (unsigned long long)request->offset, request->length);
    fflush(log_file);

    size_t size = strlen(file->data);
    if (request->offset >= size) {
        response->bytes_written = 0;
        return FP_OK;
    }

    size_t available = size - (size_t)request->offset;
    size_t count = request->length < available ? request->length : available;
    if (count > response->buffer_len) {
        count = response->buffer_len;
    }

    memcpy(response->buffer, file->data + request->offset, count);
    response->bytes_written = count;
    return FP_OK;
}

static void fail(const char *label, fp_status_t status) {
    fprintf(stderr, "%s: %s (%u)\n", label, fp_status_string(status), status);
    exit(1);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: read_only_mvp_provider <read-log>\n");
        return 2;
    }

    const char *runtime_dir = getenv("XDG_RUNTIME_DIR");
    if (runtime_dir == NULL) {
        fprintf(stderr, "XDG_RUNTIME_DIR is required\n");
        return 2;
    }

    FILE *log_file = fopen(argv[1], "a");
    if (log_file == NULL) {
        perror("fopen");
        return 1;
    }

    signal(SIGTERM, stop_provider);
    signal(SIGINT, stop_provider);

    fp_context_options_t options = FP_CONTEXT_OPTIONS_INIT;
    options.runtime_dir = runtime_dir;
    fp_context_t *context = NULL;
    fp_status_t status = fp_context_open(&options, &context);
    if (status != FP_OK) {
        fail("fp_context_open", status);
    }

    fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_file);
    fp_provider_t *provider = NULL;
    status = fp_provider_register(context, &ops, log_file, &provider);
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
    status = fp_promise_add_dir(builder, "docs", &dir_attr, "remote-dir-1");
    if (status != FP_OK) {
        fail("fp_promise_add_dir", status);
    }
    status =
        fp_promise_add_dir(builder, "docs/guides", &dir_attr, "remote-dir-2");
    if (status != FP_OK) {
        fail("fp_promise_add_dir nested", status);
    }
    status = fp_promise_add_dir(builder, "docs/empty", &dir_attr,
                                "remote-dir-empty");
    if (status != FP_OK) {
        fail("fp_promise_add_dir empty", status);
    }

    fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
    file_attr.mode = 0644;
    file_attr.size = strlen(kFiles[0].data);
    status = fp_promise_add_file(builder, "docs/readme.txt", &file_attr,
                                 "remote-file-1");
    if (status != FP_OK) {
        fail("fp_promise_add_file", status);
    }
    file_attr.size = strlen(kFiles[1].data);
    status = fp_promise_add_file(builder, "docs/guides/setup.txt", &file_attr,
                                 "remote-file-2");
    if (status != FP_OK) {
        fail("fp_promise_add_file nested", status);
    }
    file_attr.size = strlen(kFiles[2].data);
    status = fp_promise_add_file(builder, "pending.txt", &file_attr,
                                 "remote-file-3");
    if (status != FP_OK) {
        fail("fp_promise_add_file pending", status);
    }

    char path[4096];
    status = fp_promise_commit(builder, path, sizeof(path));
    if (status != FP_OK) {
        fail("fp_promise_commit", status);
    }

    printf("visible_path=%s\n", path);
    fflush(stdout);

    while (keep_running) {
        sleep(1);
    }

    fp_promise_builder_free(builder);
    fp_provider_unregister(provider);
    fp_context_close(context);
    fclose(log_file);
    return 0;
}
