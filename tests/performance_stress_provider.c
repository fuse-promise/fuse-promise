#include <fuse-promise/fuse-promise.h>

#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define TREE_DIRS 12
#define TREE_FILES_PER_DIR 25
#define TREE_FILE_SIZE 16
#define LARGE_FILE_SIZE 262144

static volatile sig_atomic_t keep_running = 1;
static const char kPattern[] = "0123456789abcdef";

static void stop_provider(int signal_number) {
    (void)signal_number;
    keep_running = 0;
}

static void fill_pattern(char *buffer, size_t offset, size_t count) {
    size_t pattern_len = strlen(kPattern);
    for (size_t index = 0; index < count; index++) {
        buffer[index] = kPattern[(offset + index) % pattern_len];
    }
}

static int is_registered_tree_file(const char *relative_path,
                                   const char *node_id) {
    unsigned int path_dir = 0;
    unsigned int path_file = 0;
    char trailing = '\0';
    int parsed_path = sscanf(relative_path, "tree/dir-%u/file-%u.txt%c",
                             &path_dir, &path_file, &trailing);
    if (parsed_path != 2 || path_dir >= TREE_DIRS ||
        path_file >= TREE_FILES_PER_DIR) {
        return 0;
    }

    char expected_path[128];
    char expected_node_id[128];
    snprintf(expected_path, sizeof(expected_path), "tree/dir-%02u/file-%02u.txt",
             path_dir, path_file);
    snprintf(expected_node_id, sizeof(expected_node_id), "tree-file-%02u-%02u",
             path_dir, path_file);
    return strcmp(relative_path, expected_path) == 0 &&
           strcmp(node_id, expected_node_id) == 0;
}

static fp_status_t read_generated_file(const fp_read_request_t *request,
                                       fp_read_response_t *response,
                                       void *user_data) {
    FILE *log_file = (FILE *)user_data;
    if (request == NULL || response == NULL || response->buffer == NULL ||
        log_file == NULL) {
        return FP_ERR_INVALID_ARGUMENT;
    }

    size_t file_size = 0;
    if (strcmp(request->relative_path, "large.bin") == 0 &&
        strcmp(request->node_id, "large-file") == 0) {
        file_size = LARGE_FILE_SIZE;
    } else if (is_registered_tree_file(request->relative_path,
                                       request->node_id)) {
        file_size = TREE_FILE_SIZE;
    } else {
        return FP_ERR_INVALID_ARGUMENT;
    }

    fprintf(log_file, "READ path=%s offset=%llu length=%zu\n",
            request->relative_path, (unsigned long long)request->offset,
            request->length);
    fflush(log_file);

    if (request->offset >= file_size) {
        response->bytes_written = 0;
        return FP_OK;
    }

    size_t available = file_size - (size_t)request->offset;
    size_t count = request->length < available ? request->length : available;
    if (count > response->buffer_len) {
        count = response->buffer_len;
    }

    fill_pattern((char *)response->buffer, (size_t)request->offset, count);
    response->bytes_written = count;
    return FP_OK;
}

static void fail(const char *label, fp_status_t status) {
    fprintf(stderr, "%s: %s (%u)\n", label, fp_status_string(status), status);
    exit(1);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: performance_stress_provider <read-log>\n");
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

    fp_provider_ops_t ops = FP_PROVIDER_OPS_INIT(read_generated_file);
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
    status = fp_promise_add_dir(builder, "tree", &dir_attr, "tree-root");
    if (status != FP_OK) {
        fail("fp_promise_add_dir tree", status);
    }

    char path[128];
    char node_id[128];
    for (size_t dir_index = 0; dir_index < TREE_DIRS; dir_index++) {
        snprintf(path, sizeof(path), "tree/dir-%02zu", dir_index);
        snprintf(node_id, sizeof(node_id), "tree-dir-%02zu", dir_index);
        status = fp_promise_add_dir(builder, path, &dir_attr, node_id);
        if (status != FP_OK) {
            fail("fp_promise_add_dir generated", status);
        }
    }

    fp_node_attr_t file_attr = FP_NODE_ATTR_INIT;
    file_attr.mode = 0644;
    file_attr.size = TREE_FILE_SIZE;
    for (size_t dir_index = 0; dir_index < TREE_DIRS; dir_index++) {
        for (size_t file_index = 0; file_index < TREE_FILES_PER_DIR;
             file_index++) {
            snprintf(path, sizeof(path), "tree/dir-%02zu/file-%02zu.txt",
                     dir_index, file_index);
            snprintf(node_id, sizeof(node_id), "tree-file-%02zu-%02zu",
                     dir_index, file_index);
            status = fp_promise_add_file(builder, path, &file_attr, node_id);
            if (status != FP_OK) {
                fail("fp_promise_add_file generated", status);
            }
        }
    }

    file_attr.size = LARGE_FILE_SIZE;
    status = fp_promise_add_file(builder, "large.bin", &file_attr,
                                 "large-file");
    if (status != FP_OK) {
        fail("fp_promise_add_file large", status);
    }

    char visible_path[4096];
    status = fp_promise_commit(builder, visible_path, sizeof(visible_path));
    if (status != FP_OK) {
        fail("fp_promise_commit", status);
    }

    printf("visible_path=%s\n", visible_path);
    printf("tree_files=%d\n", TREE_DIRS * TREE_FILES_PER_DIR);
    printf("large_file_size=%d\n", LARGE_FILE_SIZE);
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
