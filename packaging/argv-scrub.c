/* argv-scrub.so — let wrapper main parse `-L user:pass` without that
 * string ever sitting in the kernel argv area /proc/<pid>/cmdline reads.
 *
 * Upstream (wrapper.ggo, main.c on tag wrapper.x86_64.latest) accepts the
 * Apple ID only as `-L`/`--login`. wrapper-rootless.c parses argv, forks,
 * chroots, then execve("/system/bin/main", argv, envp). The kernel copies
 * those strings into the new image's cmdline, so zeroing them afterwards
 * always leaves a window, and a glibc preload does not run early enough
 * inside the bionic image to close it.
 *
 * melbad therefore execs with a placeholder (`-L @`) and the real
 * `user:pass` in a memfd at fd 250 (MELBA_LOGIN_FD), inherited across
 * `run` (sh), the glibc launcher, and bionic main. This initializer
 * repoints the `-L` value at a private buffer filled from that fd
 * (pread at offset 0, so every stage can read it). It does not write the
 * secret back into the original argv strings.
 *
 * execve is interposed. When the credential came from the fd, the value
 * passed across exec is the placeholder again, so the next image's
 * cmdline stays clean and its own copy of this .so refills the pointer.
 * Both glibc and bionic call .init_array with (argc, argv, envp).
 *
 * Init also flips MELBA_LOGIN_APPLIED=0 to =1 in the env block after a
 * successful repoint (melbad watches /proc/<pid>/environ). execve passes
 * a fresh =0 so a later image that never runs this .so stays unconfirmed.
 *
 * If fd 250 has no credential, fall back to the old in-place scrub so a
 * hand-run `wrapper -L user:pass` is unchanged. That fallback still has
 * a cmdline window; melbad does not use it.
 *
 * Build (freestanding, SysV hash — bionic rejects GNU-hash-only):
 *   cc -O2 -fPIC -shared -nostdlib -fno-stack-protector -fno-builtin \
 *      -o argv-scrub.so argv-scrub.c -Wl,--hash-style=sysv -Wl,-z,noexecstack
 */

enum { MAXSEC = 4096, MELBA_LOGIN_FD = 250 };

static const char melba_login_fd_marker[] __attribute__((used)) = "melba-login-fd-v2";

static char flag_buf[4];
static char value_buf[MAXSEC];
static char combo_buf[16 + MAXSEC];
static char placeholder[] = "@";
static char login_at[] = "--login=@";
/* Parent sets this to 0. Init flips it to 1 in place (visible in
 * /proc/<pid>/environ) only after the login argument has been repointed.
 * execve passes a fresh "=0" so the next image stays 0 until its own init. */
static char applied_off[] = "MELBA_LOGIN_APPLIED=0";
static const char applied_key[] = "MELBA_LOGIN_APPLIED=";
static int from_fd;

extern int *__errno_location(void) __attribute__((weak));

static int login_flag(const char *s) {
    if (!s || s[0] != '-' || !s[1]) return 0;
    if (s[1] == 'L' && s[2] == 0) return 1;                 /* -L */
    if (s[1] == '-' && s[2] == 'l' && s[3] == 'o' && s[4] == 'g'
        && s[5] == 'i' && s[6] == 'n'
        && (s[7] == 0 || s[7] == '=')) return 2;            /* --login[=] */
    return 0;
}

#if defined(__x86_64__)
#define SYS_pread64 17
#define SYS_execve 59
static long sc3(long nr, long a, long b, long c) {
    long ret;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(nr), "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory");
    return ret;
}
static long sc4(long nr, long a, long b, long c, long d) {
    long ret;
    register long r10 __asm__("r10") = d;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10)
                     : "rcx", "r11", "memory");
    return ret;
}
#elif defined(__aarch64__)
#define SYS_pread64 67
#define SYS_execve 221
static long sc3(long nr, long a, long b, long c) {
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}
static long sc4(long nr, long a, long b, long c, long d) {
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    __asm__ volatile("svc #0"
                     : "+r"(x0)
                     : "r"(x8), "r"(x1), "r"(x2), "r"(x3)
                     : "memory");
    return x0;
}
#else
#error "argv-scrub.so needs x86_64 or aarch64"
#endif

static void zap(char *s) {
    volatile char *p = (volatile char *)s;
    while (*p) *p++ = 0;
}

static int load_from_fd(void) {
    char tmp[MAXSEC];
    long n;
    long i;
    if (from_fd) return 1;
    /* A full buffer means the credential did not fit — refuse it rather
     * than authenticate a truncated prefix. */
    n = sc4(SYS_pread64, MELBA_LOGIN_FD, (long)tmp, MAXSEC, 0);
    if (n <= 0 || n >= MAXSEC) return 0;
    for (i = 0; i < n; i++) value_buf[i] = tmp[i];
    value_buf[n] = 0;
    from_fd = 1;
    (void)melba_login_fd_marker;
    return 1;
}

static char *copy_and_zap(const char *src) {
    int n = 0;
    while (src[n] && n < MAXSEC - 1) {
        value_buf[n] = src[n];
        n++;
    }
    value_buf[n] = 0;
    zap((char *)src);
    return value_buf;
}

static int same_secret(const char *s) {
    const char *a = s;
    const char *b = value_buf;
    if (!s) return 0;
    while (*a && *b && *a == *b) {
        a++;
        b++;
    }
    return *a == 0 && *b == 0;
}

static int is_applied_var(const char *s) {
    const char *k = applied_key;
    if (!s) return 0;
    while (*k && *s && *k == *s) {
        k++;
        s++;
    }
    return *k == 0;
}

/* Flip MELBA_LOGIN_APPLIED=0 to =1 inside the kernel env area. */
static void mark_applied(char **envp) {
    int k;
    if (!envp) return;
    for (; *envp; envp++) {
        char *s = *envp;
        k = 0;
        while (applied_key[k] && s[k] == applied_key[k]) k++;
        if (applied_key[k] == 0 && s[k] && s[k + 1] == 0) {
            s[k] = '1';
            return;
        }
    }
}

__attribute__((used))
void argv_scrub_init(int argc, char **argv, char **envp) {
    int have;
    int i;
    have = load_from_fd();
    for (i = 1; i < argc; i++) {
        int kind = login_flag(argv[i]);
        if (!kind) continue;
        if (kind == 2) {
            char *s = argv[i];
            while (*s && *s != '=') s++;
            if (*s == '=') {
                if (have) {
                    const char *pfx = "--login=";
                    const char *v = value_buf;
                    char *d = combo_buf;
                    char *end = combo_buf + sizeof(combo_buf) - 1;
                    while (*pfx && d < end) *d++ = *pfx++;
                    while (*v && d < end) *d++ = *v++;
                    *d = 0;
                    argv[i] = combo_buf;
                    mark_applied(envp);
                } else {
                    const char *pfx = "--login=";
                    const char *v = s + 1;
                    char *d = combo_buf;
                    char *end = combo_buf + sizeof(combo_buf) - 1;
                    while (*pfx && d < end) *d++ = *pfx++;
                    while (*v && d < end) *d++ = *v++;
                    *d = 0;
                    zap(argv[i]);
                    argv[i] = combo_buf;
                }
            } else if (i + 1 < argc && argv[i + 1][0] != '-') {
                argv[i + 1] = have ? value_buf : copy_and_zap(argv[i + 1]);
                if (have) mark_applied(envp);
            }
        } else if (i + 1 < argc) {
            argv[i + 1] = have ? value_buf : copy_and_zap(argv[i + 1]);
            if (have) mark_applied(envp);
            if (!have) {
                flag_buf[0] = '-';
                flag_buf[1] = 'L';
                flag_buf[2] = 0;
                zap(argv[i]);
                argv[i] = flag_buf;
            }
        }
        return;
    }
}

__attribute__((visibility("default")))
int execve(const char *path, char *const argv[], char *const envp[]) {
    char *local[256];
    char *elocal[4096];
    int n = 0;
    int m;
    int i;
    long rc;
    char **use_env;
    if (!from_fd || !argv) {
        rc = sc3(SYS_execve, (long)path, (long)argv, (long)envp);
        goto fail;
    }
    for (; argv[n] && n < 255; n++) local[n] = (char *)argv[n];
    if (argv[n]) {
        /* don't exec a truncated argv that might still hold the secret */
        rc = -7; /* E2BIG */
        goto fail;
    }
    local[n] = 0;
    for (i = 0; i < n; i++) {
        int kind = login_flag(local[i]);
        if (same_secret(local[i]) || local[i] == value_buf || local[i] == combo_buf)
            local[i] = placeholder;
        if (kind == 1 && i + 1 < n) local[i + 1] = placeholder;
        else if (kind == 2) {
            char *s = local[i];
            int eq = 0;
            while (*s) {
                if (*s == '=') eq = 1;
                s++;
            }
            if (eq) local[i] = login_at;
            else if (i + 1 < n) local[i + 1] = placeholder;
        }
    }
    use_env = (char **)envp;
    if (envp) {
        m = 0;
        for (; envp[m] && m < 4095; m++)
            elocal[m] = is_applied_var(envp[m]) ? applied_off : (char *)envp[m];
        if (envp[m]) {
            rc = -7; /* E2BIG — cannot reset APPLIED for the next image */
            goto fail;
        }
        elocal[m] = 0;
        use_env = elocal;
    }
    rc = sc3(SYS_execve, (long)path, (long)local, (long)use_env);
fail:
    if (rc < 0 && __errno_location) *__errno_location() = (int)(-rc);
    return -1;
}

__asm__(".pushsection .init_array,\"aw\"\n\t"
        ".quad argv_scrub_init\n\t"
        ".popsection");
