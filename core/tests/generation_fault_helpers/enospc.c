/* Linux-only syscall-boundary fault injection, never actual disk exhaustion.
 * Scope: exact temporary DB/WAL path, armed only after Store::open.
 * The event pipe (stdout) proves the selected write was reached. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int hit(int fd) {
    const char *arm = getenv("GENFAULT_ARM");
    const char *target = getenv("GENFAULT_TARGET");
    if (!arm || !target || access(arm, F_OK)) return 0;
    char link[64], path[PATH_MAX];
    snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(link, path, sizeof(path)-1);
    if (n < 0) return 0;
    path[n] = 0;
    if (strcmp(path, target)) return 0;
    static unsigned count;
    unsigned at = (unsigned)strtoul(getenv("GENFAULT_AT"), NULL, 10);
    if (++count < at) return 0;
    if (count == at) {
        if (write(STDOUT_FILENO, "GENFAULT_HIT\n", 13) != 13) _exit(98);
        if (!strcmp(getenv("GENFAULT_MODE"), "kill")) {
            raise(SIGSTOP);
            _exit(99);
        }
    }
    errno = ENOSPC;
    return 1;
}
ssize_t pwrite64(int fd, const void *buf, size_t n, off64_t off) {
    static ssize_t (*real_fn)(int,const void *,size_t,off64_t);
    if (!real_fn) real_fn = dlsym(RTLD_NEXT, "pwrite64");
    if (hit(fd)) return -1;
    return real_fn(fd, buf, n, off);
}
ssize_t pwrite(int fd, const void *buf, size_t n, off_t off) {
    static ssize_t (*real_fn)(int,const void *,size_t,off_t);
    if (!real_fn) real_fn = dlsym(RTLD_NEXT, "pwrite");
    if (hit(fd)) return -1;
    return real_fn(fd, buf, n, off);
}
