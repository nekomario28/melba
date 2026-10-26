/* Local authentication presentation bridge for the pinned 64-bit wrapper ABI.
 * No playback/DRM hooks. The original SDK authenticates and performs its own
 * MFA. Its dialogs cross one inherited Unix socket, never a public listener or
 * argv.
 *
 * This file is deliberately freestanding: the real payload loads it as an
 * LD_PRELOAD library whose *own* library group cannot resolve libc symbols
 * (measured: the payload's loader reports "cannot locate symbol write" for any
 * preload that imports libc). So every syscall is issued directly and symbol
 * lookup is implemented here by walking the loaded images' dynamic tables,
 * exactly like the loader would with dlsym. Build: cc -shared -fPIC -nostdlib
 * -ffreestanding -fno-builtin -fno-stack-protector -Wl,--hash-style=sysv.
 */
#include <poll.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/prctl.h>

#define STR_(x) #x
#define STR(x) STR_(x)
#define DSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface16setDialogHandlerEPFvlNSt6__ndk110shared_ptrIN17storeservicescore14ProtocolDialogEEENS2_INS_36AndroidProtocolDialogResponseHandlerEEEE
#define CSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface21setCredentialsHandlerEPFvNSt6__ndk110shared_ptrIN17storeservicescore18CredentialsRequestEEENS2_INS_33AndroidCredentialsResponseHandlerEEEE
#define RUN _ZN17storeservicescore16AuthenticateFlow3runEv
#define DHANDLE                                                                \
  "_ZN20androidstoreservices28AndroidPresentationInterface28handleProtocolDia" \
  "logResponseERKlRKNSt6__ndk110shared_"                                       \
  "ptrIN17storeservicescore22ProtocolDialogResponseEEE"
#define CHANDLE                                                                \
  "_ZN20androidstoreservices28AndroidPresentationInterface25handleCredentials" \
  "ResponseERKNSt6__ndk110shared_"                                             \
  "ptrIN17storeservicescore19CredentialsResponseEEE"

/* ------------------------------------------------------------------ syscalls */
#define SYS_READ 0
#define SYS_WRITE 1
#define SYS_CLOSE 3
#define SYS_POLL 7
#define SYS_PRCTL 157
#define SYS_EXIT_GROUP 231
#define SYS_OPENAT 257
#define AT_FDCWD_ (-100)

static long syscall_(long number, long a, long b, long c, long d, long e,
                     long f) {
  register long r10 __asm__("r10") = d;
  register long r8 __asm__("r8") = e;
  register long r9 __asm__("r9") = f;
  long result;
  __asm__ volatile("syscall"
                   : "=a"(result)
                   : "a"(number), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8),
                     "r"(r9)
                   : "rcx", "r11", "memory");
  return result;
}
static long read_(int fd, void *buffer, unsigned long size) {
  return syscall_(SYS_READ, fd, (long)buffer, (long)size, 0, 0, 0);
}
static long write_(int fd, const void *buffer, unsigned long size) {
  return syscall_(SYS_WRITE, fd, (long)buffer, (long)size, 0, 0, 0);
}
static int poll_(struct pollfd *fds, unsigned long count, int timeout_ms) {
  return (int)syscall_(SYS_POLL, (long)fds, (long)count, timeout_ms, 0, 0, 0);
}
static int prctl_(int option, long a, long b, long c, long d) {
  return (int)syscall_(SYS_PRCTL, option, a, b, c, d, 0);
}
static void exit_(int code) {
  syscall_(SYS_EXIT_GROUP, code, 0, 0, 0, 0, 0);
  for (;;)
    ;
}
static int openat_(const char *path) {
  return (int)syscall_(SYS_OPENAT, AT_FDCWD_, (long)path, 0, 0, 0, 0);
}




/* ---------------------------------------------------- no-libc string helpers */
static int same(const char *a, const char *b) {
  while (*a && *a == *b) {
    a++;
    b++;
  }
  return *a == *b;
}
static int contains(const char *text, const char *needle) {
  for (; *text; text++) {
    const char *t = text;
    const char *n = needle;
    while (*n && *t == *n) {
      t++;
      n++;
    }
    if (!*n)
      return 1;
  }
  return 0;
}
static int matches(const char *entry, const char *name, unsigned long length) {
  for (unsigned long i = 0; i < length; i++)
    if (entry[i] != name[i])
      return 0;
  return 1;
}

/* ----------------------------------------------- /proc/self query primitives */
static char environ_text[8192];
static unsigned long environ_size;
static int environ_ready;

/* The environment of the payload process, read without getenv. */
static const char *env(const char *name) {
  if (!environ_ready) {
    environ_ready = 1;
    int fd = openat_("/proc/self/environ");
    if (fd >= 0) {
      for (;;) {
        long count = read_(fd, environ_text + environ_size,
                           sizeof(environ_text) - 1 - environ_size);
        if (count <= 0)
          break;
        environ_size += (unsigned long)count;
        if (environ_size >= sizeof(environ_text) - 1)
          break;
      }
      syscall_(SYS_CLOSE, fd, 0, 0, 0, 0, 0);
      environ_text[environ_size] = 0;
    }
  }
  unsigned long wanted = 0;
  while (name[wanted])
    wanted++;
  unsigned long i = 0;
  while (i < environ_size) {
    const char *entry = environ_text + i;
    unsigned long length = 0;
    while (i + length < environ_size && entry[length])
      length++;
    if (length > wanted && entry[wanted] == '=' &&
        matches(entry, name, wanted))
      return entry + wanted + 1;
    i += length + 1;
  }
  return 0;
}

/* ------------------------------------------- loaded-image symbol resolution */
static char maps_text[98304];
static unsigned long maps_size;
static int images_ready;
struct image {
  unsigned long base;
  const char *path;
};
/* The payload maps many data files and Android shared objects; only mapped
 * ELF images can hold the symbols this bridge resolves. */
static struct image images[256];
static int image_count;

static int looks_like_elf(unsigned long base) {
  const unsigned char *header = (const unsigned char *)base;
  return header[0] == 0x7f && header[1] == 'E' && header[2] == 'L' &&
         header[3] == 'F';
}

static unsigned long hexadecimal(const char **cursor) {
  const char *p = *cursor;
  unsigned long value = 0;
  int digits = 0;
  while (*p) {
    unsigned long digit;
    if (*p >= '0' && *p <= '9')
      digit = (unsigned long)(*p - '0');
    else if (*p >= 'a' && *p <= 'f')
      digit = (unsigned long)(*p - 'a' + 10);
    else
      break;
    value = value * 16 + digit;
    p++;
    digits++;
  }
  if (!digits)
    return 0;
  *cursor = p;
  return value;
}

/* Map the ELF images the kernel has loaded for us (file offset 0 mappings). */
static void prepare_images(void) {
  if (images_ready)
    return;
  images_ready = 1;
  int fd = openat_("/proc/self/maps");
  if (fd < 0)
    return;
  for (;;) {
    long count = read_(fd, maps_text + maps_size, sizeof(maps_text) - 1 - maps_size);
    if (count <= 0)
      break;
    maps_size += (unsigned long)count;
    if (maps_size >= sizeof(maps_text) - 1)
      break;
  }
  syscall_(SYS_CLOSE, fd, 0, 0, 0, 0, 0);
  maps_text[maps_size] = 0;
  char *p = maps_text;
  while (*p && image_count < 256) {
    const char *line = p;
    while (*p && *p != '\n')
      p++;
    char *next = *p ? p + 1 : p;
    const char *cursor = line;
    hexadecimal(&cursor); /* start address */
    if (*cursor != '-') {
      p = next;
      continue;
    }
    cursor++;
    hexadecimal(&cursor); /* end address */
    while (*cursor == ' ')
      cursor++;
    const char *permissions = cursor;
    cursor += 4; /* permissions */
    while (*cursor == ' ')
      cursor++;
    unsigned long offset = hexadecimal(&cursor);
    /* The device/inode fields never contain '/', so the path starts at the
     * first slash of the remainder. */
    const char *path = cursor;
    while (*path && *path != '/' && *path != '\n')
      path++;
    if (offset == 0 && *path == '/' && permissions[0] == 'r') {
      /* Terminate the mapping path in place: the map file is line based and
       * the recorded path must not run into the following lines. */
      char *terminate = (char *)path;
      while (*terminate && *terminate != '\n')
        terminate++;
      *terminate = 0;
      const char *start = line;
      unsigned long base = hexadecimal(&start);
      if (!looks_like_elf(base)) {
        p = next;
        continue;
      }
      int known = 0;
      for (int i = 0; i < image_count; i++)
        if (same(images[i].path, path))
          known = 1;
      if (!known) {
        images[image_count].base = base;
        images[image_count].path = path;
        image_count++;
      }
    }
    p = next;
  }
}

/* Dynamic-section values are pointers: the loader has already relocated them
 * for shared objects, while a link-time offset is well below the mapping. */
static unsigned long absolute(const struct image *image, unsigned long value) {
  return value >= image->base ? value : image->base + value;
}

static unsigned long lookup(const struct image *image, const char *name) {
  unsigned char *base = (unsigned char *)image->base;
  if (base[0] != 0x7f || base[1] != 'E' || base[2] != 'L' || base[3] != 'F')
    return 0;
  if (base[4] != 2 || base[5] != 1) /* ELFCLASS64, little endian */
    return 0;
  unsigned long phoff = *(uint64_t *)(base + 32);
  unsigned long phentsize = *(uint16_t *)(base + 54);
  unsigned long phnum = *(uint16_t *)(base + 56);
  unsigned char *dynamic = 0;
  for (unsigned long i = 0; i < phnum && phentsize; i++) {
    unsigned char *header = base + phoff + i * phentsize;
    if (*(uint32_t *)header == 2 /* PT_DYNAMIC */) {
      dynamic = base + *(uint64_t *)(header + 16);
      break;
    }
  }
  if (!dynamic)
    return 0;
  unsigned long strtab = 0, symtab = 0, hash = 0, gnu_hash = 0;
  for (unsigned char *entry = dynamic;; entry += 16) {
    int64_t tag = *(int64_t *)entry;
    unsigned long value = *(uint64_t *)(entry + 8);
    if (tag == 0)
      break;
    if (tag == 5)
      strtab = absolute(image, value);
    else if (tag == 6)
      symtab = absolute(image, value);
    else if (tag == 4)
      hash = absolute(image, value);
    else if (tag == 0x6ffffef5)
      gnu_hash = absolute(image, value);
  }
  if (!strtab || !symtab)
    return 0;
  unsigned long count = 0;
  if (hash) {
    count = ((uint32_t *)hash)[1]; /* nchain */
  } else if (gnu_hash) {
    uint32_t buckets_count = ((uint32_t *)gnu_hash)[0];
    uint32_t symoffset = ((uint32_t *)gnu_hash)[1];
    uint32_t bloom_size = ((uint32_t *)gnu_hash)[2];
    uint32_t *buckets =
        (uint32_t *)((unsigned char *)gnu_hash + 16 + (unsigned long)bloom_size * 8);
    uint32_t *chain = buckets + buckets_count;
    for (uint32_t bucket = 0; bucket < buckets_count; bucket++) {
      uint32_t index = buckets[bucket];
      if (!index)
        continue;
      for (unsigned long symbol = index; symbol < 200000; symbol++) {
        uint32_t value = chain[symbol - symoffset];
        if (symbol + 1 > count)
          count = symbol + 1;
        if (value & 1)
          break;
      }
    }
  }
  if (!count || count > 200000)
    return 0;
  const char *names = (const char *)strtab;
  for (unsigned long i = 0; i < count; i++) {
    unsigned char *symbol = (unsigned char *)(symtab + i * 24);
    uint32_t name_offset = *(uint32_t *)symbol;
    uint16_t section = *(uint16_t *)(symbol + 6);
    if (!name_offset || section == 0) /* 0 is SHN_UNDEF */
      continue;
    if (same(names + name_offset, name))
      return absolute(image, *(uint64_t *)(symbol + 8));
  }
  return 0;
}

static unsigned long find_symbol(const char *name, int skip_self) {
  prepare_images();
  for (int i = 0; i < image_count; i++) {
    /* This library interposes three symbols; asking for "the original" of one
     * must never return our own definition. */
    if (skip_self && contains(images[i].path, "melba-auth-"))
      continue;
    unsigned long found = lookup(&images[i], name);
    if (found)
      return found;
  }
  return 0;
}
static void *symbol(const char *name) {
  unsigned long found = find_symbol(name, 1);
  if (!found)
    exit_(70);
  return (void *)found;
}

/* ------------------------------------------------------------------- bridge */
struct shared {
  void *obj, *control;
};
struct vector {
  struct shared *begin, *end, *capacity;
};
union string {
  struct {
    size_t cap, size, data;
  };
  unsigned char small[24];
};
static void *presentation;
static int channel = -1, busy;
static uint32_t sequence;
static unsigned char out[65536], in[65536];
static size_t used, pos, available;
static char username[1024], password[4096], submitted[4104];
static void die(void) { exit_(70); }
static void erase(void *p, size_t n) {
  volatile unsigned char *s = p;
  while (n--)
    *s++ = 0;
}
static size_t length(const char *s) {
  size_t n = 0;
  if (!s)
    return 0;
  while (s[n] && n < 8192)
    n++;
  if (n == 8192)
    die();
  return n;
}
static const char *text(void *p) {
  union string *s = p;
  if (!s)
    return "";
  return (s->small[0] & 1) ? (const char *)s->data : (char *)s->small + 1;
}
static const char *get(const char *name, void *obj) {
  return text(((void *(*)(void *))symbol(name))(obj));
}
static union string view(const char *s) {
  union string r = {.cap = 1, .size = length(s), .data = (size_t)s};
  return r;
}
static void io(int writing, void *p, size_t n) {
  unsigned char *b = p;
  while (n) {
    struct pollfd fd = {.fd = channel, .events = writing ? POLLOUT : POLLIN};
    if (poll_(&fd, 1, 300000) <= 0)
      die();
    long k = writing ? write_(channel, b, n) : read_(channel, b, n);
    if (k <= 0)
      die();
    b += (size_t)k;
    n -= (size_t)k;
  }
}
static void init(void) {
  if (channel >= 0)
    return;
  const char *s = env("MELBA_AUTH_FD");
  if (!s || !*s)
    die();
  int n = 0;
  for (; *s; s++) {
    if (*s < '0' || *s > '9' || n > 100000)
      die();
    n = n * 10 + *s - '0';
  }
  if (n < 3)
    die();
  channel = n;
  if (prctl_(PR_SET_DUMPABLE, 0L, 0L, 0L, 0L) != 0)
    die();
  uint32_t hello[2] = {4, 1};
  io(1, hello, sizeof(hello));
}
static void number(uint32_t n) {
  if (used + 4 > sizeof(out))
    die();
  for (int i = 0; i < 4; i++)
    out[used++] = (unsigned char)(n >> (i * 8));
}
static void string(const char *s) {
  size_t n = length(s);
  number((uint32_t)n);
  if (used + n > sizeof(out))
    die();
  for (size_t i = 0; i < n; i++)
    out[used++] = (unsigned char)s[i];
}
static void begin(uint32_t kind) {
  init();
  if (__atomic_exchange_n(&busy, 1, __ATOMIC_ACQUIRE))
    die();
  used = 0;
  number(kind);
  number(++sequence);
}
static void send(void) {
  uint32_t n = (uint32_t)used;
  io(1, &n, 4);
  io(1, out, used);
}
static uint32_t read_number(void) {
  if (pos + 4 > available)
    die();
  uint32_t n = 0;
  for (int i = 0; i < 4; i++)
    n |= (uint32_t)in[pos++] << (8 * i);
  return n;
}
static void read_string(char *dst, size_t capacity) {
  uint32_t n = read_number();
  if (n >= capacity || pos + n > available)
    die();
  for (uint32_t i = 0; i < n; i++) {
    if (!in[pos])
      die();
    dst[i] = (char)in[pos++];
  }
  dst[n] = 0;
}
static uint32_t receive(void) {
  uint32_t n;
  io(0, &n, 4);
  if (n < 8 || n > sizeof(in))
    die();
  io(0, in, n);
  available = n;
  pos = 0;
  uint32_t kind = read_number();
  if (read_number() != sequence)
    die();
  return kind;
}
static void finish(void) {
  if (pos != available)
    die();
  erase(in, available);
  __atomic_store_n(&busy, 0, __ATOMIC_RELEASE);
}
static void response(struct shared *r, size_t size, const char *table,
                     const char *ctor) {
  /* Keep the SDK's allocator, control-block vtable and release operation.
   * Unlike upstream's stack response, this survives asynchronous SDK copies. */
  unsigned char *raw = ((void *(*)(size_t))symbol("_Znwm"))(size);
  if (!raw)
    die();
  erase(raw, size);
  *(void **)raw = (void **)(symbol(table)) + 2;
  r->obj = raw + 24;
  r->control = raw;
  ((void (*)(void *))symbol(ctor))(r->obj);
}
static void release(struct shared *r) {
  ((void (*)(void *))symbol(
      "_ZNSt6__ndk119__shared_weak_count16__release_sharedEv"))(r->control);
}
static void dialog(long id, struct shared *request, struct shared *handler) {
  (void)handler;
  begin(2);
  string(get("_ZNK17storeservicescore14ProtocolDialog5titleEv", request->obj));
  string(
      get("_ZNK17storeservicescore14ProtocolDialog7messageEv", request->obj));
  struct vector *v = ((void *(*)(void *))symbol(
      "_ZNK17storeservicescore14ProtocolDialog7buttonsEv"))(request->obj);
  if (!v || !v->begin || v->end < v->begin || v->end - v->begin > 16)
    die();
  uint32_t count = (uint32_t)(v->end - v->begin);
  number(count);
  for (uint32_t i = 0; i < count; i++)
    string(get("_ZNK17storeservicescore14ProtocolButton5titleEv",
               v->begin[i].obj));
  send();
  if (receive() != 10)
    die();
  uint32_t chosen = read_number();
  if (chosen >= count)
    die();
  struct shared r;
  response(&r, 72,
           "_ZTVNSt6__ndk120__shared_ptr_"
           "emplaceIN17storeservicescore22ProtocolDialogResponseENS_"
           "9allocatorIS2_EEEE",
           "_ZN17storeservicescore22ProtocolDialogResponseC1Ev");
  ((void (*)(void *, struct shared *))symbol(
      "_ZN17storeservicescore22ProtocolDialogResponse17setSelectedButtonERKNSt6"
      "__ndk110shared_ptrINS_14ProtocolButtonEEE"))(r.obj, &v->begin[chosen]);
  finish();
  ((void (*)(void *, long *, struct shared *))symbol(DHANDLE))(presentation,
                                                               &id, &r);
  release(&r);
}
static void credentials(struct shared *request, struct shared *handler) {
  (void)handler;
  begin(3);
  int code = ((uint8_t(*)(void *))symbol(
      "_ZNK17storeservicescore18CredentialsRequest28requiresHSA2VerificationCod"
      "eEv"))(request->obj);
  number((uint32_t)(code && !password[0] ? 2 : code));
  string(
      get("_ZNK17storeservicescore18CredentialsRequest5titleEv", request->obj));
  string(get("_ZNK17storeservicescore18CredentialsRequest7messageEv",
             request->obj));
  string(get("_ZNK17storeservicescore18CredentialsRequest17cancelButtonTitleEv",
             request->obj));
  string(get("_ZNK17storeservicescore18CredentialsRequest13okButtonTitleEv",
             request->obj));
  send();
  uint32_t kind = receive(), type = 2;
  if (kind == 10) {
    type = read_number();
    if (type != 0 && type != 2)
      die();
  } else if (kind == 11 && !code) {
    read_string(username, sizeof(username));
    read_string(password, sizeof(password));
  } else if ((kind == 12 || kind == 13) && code) {
    if (kind == 13) {
      read_string(username, sizeof(username));
      read_string(password, sizeof(password));
    }
    if (!username[0] || !password[0])
      die();
    char value[7];
    read_string(value, sizeof(value));
    if (length(value) != 6)
      die();
    for (int i = 0; i < 6; i++)
      if (value[i] < '0' || value[i] > '9')
        die();
    size_t n = length(password);
    if (n + 7 > sizeof(submitted))
      die();
    for (size_t i = 0; i < n; i++)
      submitted[i] = password[i];
    for (int i = 0; i < 7; i++)
      submitted[n + i] = value[i];
    erase(value, sizeof(value));
  } else
    die();
  struct shared r;
  response(
      &r, 80,
      "_ZTVNSt6__ndk120__shared_ptr_"
      "emplaceIN17storeservicescore19CredentialsResponseENS_9allocatorIS2_EEEE",
      "_ZN17storeservicescore19CredentialsResponseC1Ev");
  union string user = view(username),
               pass = view(code && (kind == 12 || kind == 13) ? submitted
                                                              : password);
  ((void (*)(void *, union string *))symbol(
      "_ZN17storeservicescore19CredentialsResponse11setUserNameERKNSt6__"
      "ndk112basic_stringIcNS1_11char_traitsIcEENS1_9allocatorIcEEEE"))(r.obj,
                                                                        &user);
  ((void (*)(void *, union string *))symbol(
      "_ZN17storeservicescore19CredentialsResponse11setPasswordERKNSt6__"
      "ndk112basic_stringIcNS1_11char_traitsIcEENS1_9allocatorIcEEEE"))(r.obj,
                                                                        &pass);
  ((void (*)(void *, uint32_t))symbol(
      "_ZN17storeservicescore19CredentialsResponse15setResponseTypeENS0_"
      "12ResponseTypeE"))(r.obj, type);
  finish();
  erase(submitted, sizeof(submitted));
  ((void (*)(void *, struct shared *))symbol(CHANDLE))(presentation, &r);
  release(&r);
}
void DSET(void *p, void (*callback)(long, struct shared *, struct shared *)) {
  (void)callback;
  init();
  presentation = p;
  void *real = (void *)find_symbol(STR(DSET), 1);
  if (!real)
    die();
  ((void (*)(void *, void (*)(long, struct shared *, struct shared *)))real)(
      p, dialog);
}
void CSET(void *p, void (*callback)(struct shared *, struct shared *)) {
  (void)callback;
  init();
  presentation = p;
  void *real = (void *)find_symbol(STR(CSET), 1);
  if (!real)
    die();
  ((void (*)(void *, void (*)(struct shared *, struct shared *)))real)(
      p, credentials);
}
void RUN(void *flow) {
  /* A new GUI attempt must not silently authenticate a cached other account. */
  const char *reuse = env("MELBA_AUTH_REUSE");
  ((void (*)(void *, uint8_t))symbol(
      "_ZN17storeservicescore16AuthenticateFlow14setUseExistingEb"))(
      flow, reuse && reuse[0] == '1' && !reuse[1]);
  void *real = (void *)find_symbol(STR(RUN), 1);
  if (!real)
    die();
  ((void (*)(void *))real)(flow);
  struct shared *r = ((void *(*)(void *))symbol(
      "_ZNK17storeservicescore16AuthenticateFlow8responseEv"))(flow);
  if (!r || !r->obj)
    die();
  uint32_t result = ((int (*)(void *))symbol(
      "_ZNK17storeservicescore20AuthenticateResponse12responseTypeEv"))(r->obj);
  if (result != 6)
    die();
  begin(4);
  number(result);
  send();
  erase(username, sizeof(username));
  erase(password, sizeof(password));
  __atomic_store_n(&busy, 0, __ATOMIC_RELEASE);
}
