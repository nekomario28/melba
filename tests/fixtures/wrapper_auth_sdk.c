/* Synthetic SDK edge only. No Apple libraries, accounts, or network. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
struct shared {
  void *obj, *control;
};
struct vector {
  struct shared *begin, *end, *capacity;
};
union string {
  struct {
    size_t cap, size;
    const char *data;
  };
  unsigned char small[24];
};
struct response {
  char *user, *pass;
  uint32_t type;
  int *selected;
};
#define DSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface16setDialogHandlerEPFvlNSt6__ndk110shared_ptrIN17storeservicescore14ProtocolDialogEEENS2_INS_36AndroidProtocolDialogResponseHandlerEEEE
#define CSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface21setCredentialsHandlerEPFvNSt6__ndk110shared_ptrIN17storeservicescore18CredentialsRequestEEENS2_INS_33AndroidCredentialsResponseHandlerEEEE
#define RUN _ZN17storeservicescore16AuthenticateFlow3runEv
static void (*dialog)(long, struct shared *, struct shared *);
static void (*credentials)(struct shared *, struct shared *);
static int need_code, result, chosen = -1, accepted;
static const char *scenario;
static const char *user = "audit@example.invalid",
                  *password = "fixture-password";
static void trace(const char *text) {
  FILE *f = fopen(getenv("FAKE_TRACE"), "a");
  if (!f)
    abort();
  fprintf(f, "%s\n", text);
  fclose(f);
}
static const char *value(union string *s) {
  return s->small[0] & 1 ? s->data : (char *)s->small + 1;
}
static union string *text(const char *s) {
  static union string r;
  r.cap = 1;
  r.size = strlen(s);
  r.data = s;
  return &r;
}
void DSET(void *p, void (*cb)(long, struct shared *, struct shared *)) {
  (void)p;
  dialog = cb;
}
void CSET(void *p, void (*cb)(struct shared *, struct shared *)) {
  (void)p;
  credentials = cb;
}
void _ZN17storeservicescore16AuthenticateFlow14setUseExistingEb(void *f,
                                                                uint8_t v) {
  (void)f;
  int reuse = !strcmp(getenv("FAKE_SCENARIO"), "resume");
  if (v != reuse)
    abort();
  trace(reuse ? "cached-native-flow" : "fresh-native-flow");
}
static void *allocation;
static size_t allocated_size;
static struct shared held;
void *_Znwm(size_t n) {
  if (allocation || (n != 72 && n != 80))
    abort();
  allocated_size = n;
  return allocation = calloc(1, n);
}
static void retain(struct shared *s) {
  if (s->control != allocation || s->obj != (char *)allocation + 24)
    abort();
  held = *s;
  ++*(int64_t *)((char *)s->control + 8);
}
void *
    _ZTVNSt6__ndk120__shared_ptr_emplaceIN17storeservicescore19CredentialsResponseENS_9allocatorIS2_EEEE
        [6];
void *
    _ZTVNSt6__ndk120__shared_ptr_emplaceIN17storeservicescore22ProtocolDialogResponseENS_9allocatorIS2_EEEE
        [6];
void _ZNSt6__ndk119__shared_weak_count16__release_sharedEv(void *p) {
  if (p != allocation)
    abort();
  if (--*(int64_t *)((char *)p + 8) >= 0) {
    trace("sdk-response-retained");
    return;
  }
  struct response *r = (void *)((char *)p + 24);
  free(r->user);
  free(r->pass);
  free(p);
  allocation = 0;
  trace("sdk-response-released");
}
void _ZN17storeservicescore19CredentialsResponseC1Ev(void *p) {
  if (allocated_size != 80)
    abort();
  memset(p, 0, 56);
}
void _ZN17storeservicescore22ProtocolDialogResponseC1Ev(void *p) {
  if (allocated_size != 72)
    abort();
  memset(p, 0, 48);
}
void _ZN17storeservicescore19CredentialsResponse11setUserNameERKNSt6__ndk112basic_stringIcNS1_11char_traitsIcEENS1_9allocatorIcEEEE(
    void *p, union string *s) {
  ((struct response *)p)->user = strdup(value(s));
}
void _ZN17storeservicescore19CredentialsResponse11setPasswordERKNSt6__ndk112basic_stringIcNS1_11char_traitsIcEENS1_9allocatorIcEEEE(
    void *p, union string *s) {
  ((struct response *)p)->pass = strdup(value(s));
}
void _ZN17storeservicescore19CredentialsResponse15setResponseTypeENS0_12ResponseTypeE(
    void *p, uint32_t t) {
  ((struct response *)p)->type = t;
}
uint8_t
_ZNK17storeservicescore18CredentialsRequest28requiresHSA2VerificationCodeEv(
    void *p) {
  (void)p;
  return need_code;
}
void *_ZNK17storeservicescore18CredentialsRequest5titleEv(void *p) {
  (void)p;
  return text(need_code ? "Verification code" : "Apple ID");
}
void *_ZNK17storeservicescore18CredentialsRequest7messageEv(void *p) {
  (void)p;
  return text(need_code ? "Enter the code for this synthetic challenge"
                        : "Synthetic credentials request");
}
void *
_ZNK17storeservicescore18CredentialsRequest17cancelButtonTitleEv(void *p) {
  (void)p;
  return text("Cancel");
}
void *_ZNK17storeservicescore18CredentialsRequest13okButtonTitleEv(void *p) {
  (void)p;
  return text("Continue");
}
void *_ZNK17storeservicescore14ProtocolDialog5titleEv(void *p) {
  (void)p;
  return text("Verification delivery");
}
void *_ZNK17storeservicescore14ProtocolDialog7messageEv(void *p) {
  (void)p;
  return text("Choose an offered synthetic method");
}
static int indices[] = {0, 1, 2};
static struct shared buttons[] = {
    {&indices[0], 0}, {&indices[1], 0}, {&indices[2], 0}};
static struct vector options = {buttons, buttons + 3, buttons + 3};
void *_ZNK17storeservicescore14ProtocolDialog7buttonsEv(void *p) {
  (void)p;
  return &options;
}
void *_ZNK17storeservicescore14ProtocolButton5titleEv(void *p) {
  const char *labels[] = {"Cancel", "Text ••12", "Call ••12"};
  return text(labels[*(int *)p]);
}
void _ZN17storeservicescore22ProtocolDialogResponse17setSelectedButtonERKNSt6__ndk110shared_ptrINS_14ProtocolButtonEEE(
    void *p, struct shared *b) {
  ((struct response *)p)->selected = b->obj;
}
void _ZN20androidstoreservices28AndroidPresentationInterface28handleProtocolDialogResponseERKlRKNSt6__ndk110shared_ptrIN17storeservicescore22ProtocolDialogResponseEEE(
    void *p, long *id, struct shared *s) {
  (void)p;
  if (*id != 99)
    abort();
  retain(s);
}
static void consume_dialog(void) {
  struct response *r = held.obj;
  if (!r->selected)
    abort();
  chosen = *r->selected;
  trace(chosen == 1   ? "sms-request-returned"
        : chosen == 2 ? "voice-request-returned"
                      : "cancel-returned");
  _ZNSt6__ndk119__shared_weak_count16__release_sharedEv(held.control);
}
void _ZN20androidstoreservices28AndroidPresentationInterface25handleCredentialsResponseERKNSt6__ndk110shared_ptrIN17storeservicescore19CredentialsResponseEEE(
    void *p, struct shared *s) {
  (void)p;
  retain(s);
}
static void consume_credentials(void) {
  struct response *r = held.obj;
  accepted = 0;
  if (r->type != 2 || strcmp(r->user, user))
    abort();
  char expected[128];
  snprintf(expected, sizeof(expected), "%s%s", password,
           need_code ? "654321" : "");
  accepted = !strcmp(r->pass, expected);
  trace(accepted ? "credential-response-accepted"
                 : "credential-response-rejected");
  _ZNSt6__ndk119__shared_weak_count16__release_sharedEv(held.control);
}
void *_ZNK17storeservicescore16AuthenticateFlow8responseEv(void *p) {
  (void)p;
  static struct shared r;
  r.obj = &result;
  return &r;
}
int _ZNK17storeservicescore20AuthenticateResponse12responseTypeEv(void *p) {
  return *(int *)p;
}
void RUN(void *p) {
  (void)p;
  scenario = getenv("FAKE_SCENARIO");
  if (!strcmp(scenario, "resume")) {
    result = 6;
    trace("owned-native-authentication-success");
    return;
  }
  struct shared request = {(void *)1, 0};
  if (strcmp(scenario, "cached-code")) {
    need_code = 0;
    credentials(&request, &request);
    consume_credentials();
    if (!accepted)
      return;
  }
  if (!strcmp(scenario, "native-rejected"))
    return;
  if (strcmp(scenario, "device")) {
    dialog(99, &request, &request);
    consume_dialog();
    if (chosen != 1 && chosen != 2)
      return;
    if (!strcmp(scenario, "delivery-rejected"))
      return;
  }
  need_code = 1;
  credentials(&request, &request);
  consume_credentials();
  if (!accepted) {
    credentials(&request, &request);
    consume_credentials();
    if (!accepted)
      return;
  }
  result = 6;
  trace("owned-native-authentication-success");
}
