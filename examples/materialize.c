#include <fuse-promise/fuse-promise.h>

#include <stdio.h>

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr,
                "usage: materialize <runtime-dir> <promise-path> <target-dir>\n");
        return 2;
    }

    fp_context_options_t context_options = FP_CONTEXT_OPTIONS_INIT;
    context_options.runtime_dir = argv[1];

    fp_context_t *context = NULL;
    fp_status_t status = fp_context_open(&context_options, &context);
    if (status != FP_OK) {
        fprintf(stderr, "fp_context_open: %s (%u)\n",
                fp_status_string(status), status);
        return 1;
    }

    fp_materialize_options_t materialize_options =
        FP_MATERIALIZE_OPTIONS_INIT;
    status = fp_materialize(context, argv[2], argv[3], &materialize_options);
    if (status != FP_OK) {
        fprintf(stderr, "fp_materialize: %s (%u)\n", fp_status_string(status),
                status);
        fp_context_close(context);
        return 1;
    }

    fp_context_close(context);
    return 0;
}
