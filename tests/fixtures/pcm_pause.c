/* Deterministic ALSA fault injection over null: no sound hardware is touched. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <string.h>

/* Optional physical-device probe: log actual kernel ring accounting and the
 * decoder's subsequent file seek without replacing either value. */
off64_t lseek64(int fd, off64_t offset, int whence) {
    off64_t (*seek_file)(int, off64_t, int) = dlsym(RTLD_NEXT, "lseek64");
    off64_t result = seek_file(fd, offset, whence);
    const char *trace = getenv("MELBA_TEST_SEEK_TRACE");
    if (trace) {
        char link[64], path[4096];
        snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
        ssize_t n = readlink(link, path, sizeof(path) - 1);
        if (n > 0) {
            path[n] = '\0';
            if (strstr(path, "/pause.wav")) {
                FILE *out = fopen(trace, "a");
                if (out) { fprintf(out, "seek offset=%lld whence=%d result=%lld\n", (long long)offset, whence, (long long)result); fclose(out); }
            }
        }
    }
    return result;
}

typedef struct _snd_pcm snd_pcm_t;
long snd_pcm_avail(snd_pcm_t *pcm) {
    const char *queued = getenv("MELBA_TEST_PAUSED_QUEUE_FRAMES");
    if (queued) {
        unsigned long buffer, period;
        int (*get_params)(snd_pcm_t *, unsigned long *, unsigned long *) =
            dlsym(RTLD_NEXT, "snd_pcm_get_params");
        if (get_params(pcm, &buffer, &period) < 0) return -EIO;
        FILE *out = fopen(getenv("MELBA_TEST_PAUSE_TRACE"), "a");
        if (out) { fprintf(out, "queued %s\n", queued); fclose(out); }
        return (long)buffer - strtol(queued, NULL, 10);
    }
    long (*avail)(snd_pcm_t *) = dlsym(RTLD_NEXT, "snd_pcm_avail");
    long available = avail(pcm);
    const char *trace = getenv("MELBA_TEST_SEEK_TRACE");
    if (trace) {
        unsigned long buffer = 0, period = 0;
        long delay = -1;
        int (*get_params)(snd_pcm_t *, unsigned long *, unsigned long *) = dlsym(RTLD_NEXT, "snd_pcm_get_params");
        int (*get_delay)(snd_pcm_t *, long *) = dlsym(RTLD_NEXT, "snd_pcm_delay");
        int params_rc = get_params(pcm, &buffer, &period);
        int delay_rc = get_delay(pcm, &delay);
        FILE *out = fopen(trace, "a");
        if (out) { fprintf(out, "snapshot avail=%ld buffer=%lu delay=%ld params_rc=%d delay_rc=%d\n", available, buffer, delay, params_rc, delay_rc); fclose(out); }
    }
    return available;
}
long snd_pcm_writei(snd_pcm_t *pcm, const void *buffer, unsigned long frames) {
    long (*writei)(snd_pcm_t *, const void *, unsigned long) =
        dlsym(RTLD_NEXT, "snd_pcm_writei");
    /* null normally runs faster than realtime, hiding transport transitions. */
    if (getenv("MELBA_TEST_PCM_THROTTLE")) {
        struct timespec delay = {0, (long)(frames * 1000000000ULL / 48000)};
        nanosleep(&delay, NULL);
    }
    return writei(pcm, buffer, frames);
}
int snd_pcm_pause(snd_pcm_t *pcm, int enabled) {
    const char *mode = getenv("MELBA_TEST_PAUSE_FAILURE");
    if (mode && (mode[0] == 'p' || !enabled)) {
        FILE *out = fopen(getenv("MELBA_TEST_PAUSE_TRACE"), "a");
        if (out) { fprintf(out, "reject %d\n", enabled); fclose(out); }
        if (!enabled) {
            /* A failed ioctl may invalidate the old ring-buffer accounting. */
            int (*drop_pcm)(snd_pcm_t *) = dlsym(RTLD_NEXT, "snd_pcm_drop");
            drop_pcm(pcm);
        }
        return -ENOSYS;
    }
    int (*pause_pcm)(snd_pcm_t *, int) = dlsym(RTLD_NEXT, "snd_pcm_pause");
    return pause_pcm(pcm, enabled);
}
