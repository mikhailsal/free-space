/* A prewrite barrier and observer only: never changes the real syscall result. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static int selected(int fd, long long offset) {
    if (!getenv("VOLUME_ARM") || access(getenv("VOLUME_ARM"), F_OK)) return 0;
    char link[64], path[PATH_MAX];
    snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(link, path, sizeof(path)-1);
    if (n < 0) return 0;
    path[n] = 0;
    const char *target = getenv("VOLUME_TARGET");
    if (!strcmp(target, "stage")) {
        const char *dir = getenv("VOLUME_DIR");
        size_t prefix = strlen(dir);
        if (strncmp(path, dir, prefix) || path[prefix] != '/') return 0;
        const char *name = path + prefix + 1;
        size_t len = strlen(name);
        if (strncmp(name,"scan-",5) || strchr(name,'/') || len <= 15 ||
            strcmp(name + len - 10, ".stage.tmp")) return 0;
    } else if (strcmp(path, target)) return 0;
    return offset >= strtoll(getenv("VOLUME_MIN_OFFSET"), NULL, 10);
}
static void barrier(void) {
    static int reached;
    if (reached++) return;
    if (write(1,"VOLUME_READY\n",13) != 13) _exit(98);
    char byte;
    if (read(0,&byte,1) != 1 || byte != 'x') _exit(99);
}
static void observe(ssize_t result, int saved_errno) {
    if (result < 0 && saved_errno == ENOSPC) {
        if (write(1,"REAL_ENOSPC\n",12) != 12) _exit(97);
    }
    errno = saved_errno;
}
ssize_t pwrite64(int fd, const void *buf, size_t n, off64_t off) {
    static ssize_t (*real_fn)(int,const void *,size_t,off64_t);
    if (!real_fn) real_fn = dlsym(RTLD_NEXT,"pwrite64");
    int match = selected(fd,off);
    if (match) barrier();
    ssize_t result = real_fn(fd,buf,n,off);
    int saved_errno = errno;
    if (match) observe(result,saved_errno);
    return result;
}
ssize_t pwrite(int fd, const void *buf, size_t n, off_t off) {
    static ssize_t (*real_fn)(int,const void *,size_t,off_t);
    if (!real_fn) real_fn = dlsym(RTLD_NEXT,"pwrite");
    int match = selected(fd,off);
    if (match) barrier();
    ssize_t result = real_fn(fd,buf,n,off);
    int saved_errno = errno;
    if (match) observe(result,saved_errno);
    return result;
}
