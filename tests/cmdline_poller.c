#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int contains(const unsigned char *buf, int n, const unsigned char *needle, int m) {
    int i;
    if (m <= 0 || n < m) return 0;
    for (i = 0; i <= n - m; i++) {
        if (memcmp(buf + i, needle, (size_t)m) == 0) return 1;
    }
    return 0;
}

/* argv: poller needlefile resultfile stopfile readyfile
 * needlefile holds the credential bytes (not this process's argv). */
int main(int argc, char **argv) {
    unsigned char needle[8192];
    unsigned char buf[65536];
    int nlen;
    int fd;
    long samples = 0;
    long hits = 0;
    long saw_ph = 0;
    long saw_applied = 0;
    const unsigned char ph[] = {'-', 'L', 0, '@'};
    const char *mark = "MELBA_LOGIN_APPLIED=1";
    FILE *ready;
    FILE *out;
    if (argc != 5) return 2;
    fd = open(argv[1], O_RDONLY);
    if (fd < 0) return 2;
    nlen = (int)read(fd, needle, sizeof needle);
    close(fd);
    if (nlen <= 0) return 2;
    ready = fopen(argv[4], "w");
    if (ready) {
        fputc('1', ready);
        fclose(ready);
    }
    while (access(argv[3], F_OK) != 0) {
        DIR *d = opendir("/proc");
        struct dirent *de;
        samples++;
        if (!d) continue;
        while ((de = readdir(d))) {
            char path[80];
            int cfd;
            int n;
            int pid = atoi(de->d_name);
            if (pid <= 0) continue;
            snprintf(path, sizeof path, "/proc/%d/cmdline", pid);
            cfd = open(path, O_RDONLY);
            if (cfd < 0) continue;
            n = (int)read(cfd, buf, sizeof buf);
            close(cfd);
            if (n > 0 && contains(buf, n, needle, nlen)) hits++;
            if (n >= 4 && contains(buf, n, ph, 4)) {
                unsigned char eb[65536];
                int efd;
                int en;
                saw_ph++;
                snprintf(path, sizeof path, "/proc/%d/environ", pid);
                efd = open(path, O_RDONLY);
                if (efd < 0) continue;
                en = (int)read(efd, eb, sizeof eb);
                close(efd);
                if (en > 0 && contains(eb, en, (const unsigned char *)mark, (int)strlen(mark)))
                    saw_applied++;
            }
        }
        closedir(d);
    }
    out = fopen(argv[2], "w");
    if (!out) return 2;
    fprintf(out, "samples=%ld\nhits=%ld\nsaw_placeholder=%ld\nsaw_applied=%ld\n", samples, hits,
            saw_ph, saw_applied);
    fclose(out);
    return 0;
}
