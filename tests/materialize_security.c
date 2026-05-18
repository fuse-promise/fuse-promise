#include <fuse-promise/fuse-promise.h>

#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr,
                "usage: materialize_security <promise-path> <target-dir>\n");
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

    fp_materialize_options_t materialize_options =
        FP_MATERIALIZE_OPTIONS_INIT;
    status = fp_materialize(context, argv[1], argv[2], &materialize_options);
    fp_context_close(context);

    printf("status=%s\n", fp_status_string(status));
    return status == FP_ERR_INVALID_ARGUMENT ? 0 : 1;
}
