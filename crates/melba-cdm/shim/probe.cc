#include <dlfcn.h>
#include <cstdio>
#include <cstring>
#include "content_decryption_module.h"

static void* g_host = nullptr;
class ProbeHost : public cdm::Host_10 {
 public:
  cdm::Buffer* Allocate(uint32_t) override { return nullptr; }
  void SetTimer(int64_t, void*) override {}
  cdm::Time GetCurrentWallTime() override { return 0; }
  void OnInitialized(bool ok) override { printf("[host] OnInitialized=%d\n", ok); }
  void OnResolveKeyStatusPromise(uint32_t, cdm::KeyStatus) override {}
  void OnResolveNewSessionPromise(uint32_t, const char*, uint32_t) override {}
  void OnResolvePromise(uint32_t) override {}
  void OnRejectPromise(uint32_t, cdm::Exception, uint32_t, const char*, uint32_t) override {}
  void OnSessionMessage(const char* sid, uint32_t sids, cdm::MessageType t,
                        const char* msg, uint32_t msgs) override {
    printf("[host] OnSessionMessage sid=%.*s type=%d msg_size=%u\n", sids, sid, t, msgs);
  }
  void OnSessionKeysChange(const char*, uint32_t, bool, const cdm::KeyInformation*, uint32_t) override {}
  void OnExpirationChange(const char*, uint32_t, cdm::Time) override {}
  void OnSessionClosed(const char*, uint32_t) override {}
  void SendPlatformChallenge(const char*, uint32_t, const char*, uint32_t) override {}
  void EnableOutputProtection(uint32_t) override {}
  void QueryOutputProtectionStatus() override {}
  void OnDeferredInitializationDone(cdm::StreamType, cdm::Status) override {}
  cdm::FileIO* CreateFileIO(cdm::FileIOClient*) override { return nullptr; }
  void RequestStorageId(uint32_t) override {}
};
static void* get_cdm_host(int, void*) { return g_host; }

int main(int argc, char** argv) {
  if (argc != 2 || argv[1][0] == '\0') {
    fprintf(stderr, "Usage: %s /path/to/libwidevinecdm.so\n", argv[0]);
    return 2;
  }
  const char* so = argv[1];
  void* h = dlopen(so, RTLD_NOW);
  if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
  auto ver = (const char* (*)())dlsym(h, "GetCdmVersion");
  auto init = (void (*)())dlsym(h, "InitializeCdmModule_4");
  auto create = (void* (*)(int, const char*, uint32_t, void* (*)(int, void*), void*))dlsym(h, "CreateCdmInstance");
  printf("GetCdmVersion: %s\n", ver ? ver() : "?");
  if (init) init();
  ProbeHost host; g_host = &host;
  const char* ks = "com.widevine.alpha";
  void* cdm = create(10, ks, strlen(ks), get_cdm_host, nullptr);
  printf("CreateCdmInstance: %p\n", cdm);
  if (cdm) {
    auto* m = static_cast<cdm::ContentDecryptionModule_10*>(cdm);
    m->Initialize(false, false, false);
  }
  return 0;
}
