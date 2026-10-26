/* A glibc fake launcher. Never loads Apple code or performs network I/O. */
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
struct shared {
  void *obj, *control;
};
#define DSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface16setDialogHandlerEPFvlNSt6__ndk110shared_ptrIN17storeservicescore14ProtocolDialogEEENS2_INS_36AndroidProtocolDialogResponseHandlerEEEE
#define CSET                                                                   \
  _ZN20androidstoreservices28AndroidPresentationInterface21setCredentialsHandlerEPFvNSt6__ndk110shared_ptrIN17storeservicescore18CredentialsRequestEEENS2_INS_33AndroidCredentialsResponseHandlerEEEE
#define RUN _ZN17storeservicescore16AuthenticateFlow3runEv
extern void DSET(void *, void (*)(long, struct shared *, struct shared *));
extern void CSET(void *, void (*)(struct shared *, struct shared *));
extern void RUN(void *);
extern void *_ZNK17storeservicescore16AuthenticateFlow8responseEv(void *);
extern int
_ZNK17storeservicescore20AuthenticateResponse12responseTypeEv(void *);
int main(int argc, char **argv) {
  int private_login = 0;
  for (int i = 1; i + 1 < argc; i++)
    if (!strcmp(argv[i], "-L") && !strcmp(argv[i + 1], ":"))
      private_login = 1;
  if (!private_login)
    return 91;
  FILE *f = fopen(getenv("FAKE_PIDS"), "w");
  if (!f)
    return 92;
  pid_t extra = fork();
  if (extra == 0) {
    signal(SIGTERM, SIG_IGN);
    for (;;)
      pause();
  }
  if (extra < 0)
    return 93;
  fprintf(f, "%d %d\n", getpid(), extra);
  fclose(f);
  DSET((void *)1, 0);
  CSET((void *)1, 0);
  const char *scenario = getenv("FAKE_SCENARIO");
  if (!strcmp(scenario, "zero-exit") || !strcmp(scenario, "restore-failed"))
    return 0;
  RUN((void *)1);
  struct shared *response =
      _ZNK17storeservicescore16AuthenticateFlow8responseEv((void *)1);
  if (_ZNK17storeservicescore20AuthenticateResponse12responseTypeEv(
          response->obj) != 6 &&
      strcmp(getenv("FAKE_SCENARIO"), "native-rejected"))
    return 1;
  if (strcmp(getenv("FAKE_SCENARIO"), "stale-token")) {
    f = fopen(getenv("FAKE_TOKEN_FILE"), "w");
    if (!f)
      return 94;
    fputs("fixture-native-music-token", f);
    fclose(f);
  }
  for (;;)
    pause();
}
