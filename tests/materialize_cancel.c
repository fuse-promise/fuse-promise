#include <fuse-promise/fuse-promise.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

struct progress_state {
    uint64_t callbacks;
};

static fp_status_t cancel_after_first_entry(
    const fp_materialize_progress_t *progress,
    void *user_data) {
    if (progress == NULL || user_data == NULL ||
        progress->struct_size < sizeof(fp_materialize_progress_t) ||
        progress->target_path == NULL) {
        return FP_ERR_INVALID_ARGUMENT;
    }

    struct progress_state *state = user_data;
    state->callbacks++;
    printf("progress callbacks=%llu entries_done=%llu entries_total=%llu "
           "bytes_written=%llu bytes_total=%llu target_path=%s\n",
           (unsigned long long)state->callbacks,
           (unsigned long long)progress->entries_done,
           (unsigned long long)progress->entries_total,
           (unsigned long long)progress->bytes_written,
           (unsigned long long)progress->bytes_total,
           progress->target_path);

    if (progress->entries_done > 0) {
        return FP_ERR_CANCELLED;
    }
    return FP_OK;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: materialize_cancel <promise-path> <target-dir>\n");
        return 2;
    }

    const char *runtime_dir = getenv("XDG_RUNTIME_DIR");
    if (runtime_dir == NULL) {
        fprintf(stderr, "XDG_RUNTIME_DIR is required\n");
        return 2;
    }

    fp_context_options_t context_options = FP_CONTEXT_OPTIONS_INIT;
    context_options.runtime_dir = runtime_dir;
    fp_context_t *context = NULL;
    fp_status_t status = fp_context_open(&context_options, &context);
    if (status != FP_OK) {
        fprintf(stderr, "fp_context_open: %s (%u)\n",
                fp_status_string(status), status);
        return 1;
    }

    struct progress_state progress_state = {0};
    fp_materialize_options_t materialize_options =
        FP_MATERIALIZE_OPTIONS_INIT;
    materialize_options.progress = cancel_after_first_entry;
    materialize_options.progress_user_data = &progress_state;

    status = fp_materialize(context, argv[1], argv[2], &materialize_options);
    fp_context_close(context);

    printf("status=%s\n", fp_status_string(status));
    printf("callbacks=%llu\n", (unsigned long long)progress_state.callbacks);
    return status == FP_ERR_CANCELLED && progress_state.callbacks >= 2 ? 0 : 1;
}
