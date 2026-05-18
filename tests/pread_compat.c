#define _XOPEN_SOURCE 700

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 5) {
        fprintf(stderr, "usage: pread_compat <path> <offset> <count> <expected>\n");
        return 2;
    }

    char *end = NULL;
    unsigned long long offset = strtoull(argv[2], &end, 10);
    if (end == argv[2] || *end != '\0') {
        fprintf(stderr, "invalid offset: %s\n", argv[2]);
        return 2;
    }
    unsigned long count = strtoul(argv[3], &end, 10);
    if (end == argv[3] || *end != '\0' || count == 0) {
        fprintf(stderr, "invalid count: %s\n", argv[3]);
        return 2;
    }
    if (strlen(argv[4]) != count) {
        fprintf(stderr, "expected string length does not match count\n");
        return 2;
    }

    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    char *buffer = calloc(count + 1, 1);
    if (buffer == NULL) {
        perror("calloc");
        close(fd);
        return 1;
    }

    ssize_t read_count = pread(fd, buffer, count, (off_t)offset);
    if (read_count < 0) {
        perror("pread");
        free(buffer);
        close(fd);
        return 1;
    }
    if ((unsigned long)read_count != count) {
        fprintf(stderr, "short pread: %zd\n", read_count);
        free(buffer);
        close(fd);
        return 1;
    }
    if (memcmp(buffer, argv[4], count) != 0) {
        fprintf(stderr, "unexpected pread bytes: %s\n", buffer);
        free(buffer);
        close(fd);
        return 1;
    }

    free(buffer);
    close(fd);
    return 0;
}
