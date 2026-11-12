/* Deterministic ALSA fault injection over null: no sound hardware is touched. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

typedef struct _snd_pcm snd_pcm_t;
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
        return -ENOSYS;
    }
    int (*pause_pcm)(snd_pcm_t *, int) = dlsym(RTLD_NEXT, "snd_pcm_pause");
    return pause_pcm(pcm, enabled);
}
