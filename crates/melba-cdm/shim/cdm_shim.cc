#include <dlfcn.h>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <fstream>
#include <map>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <thread>
#include <vector>
#include "content_decryption_module.h"
#include "melba_cdm.h"

namespace {

class HeapBuffer : public cdm::Buffer {
 public:
  explicit HeapBuffer(uint32_t cap) : cap_(cap) { data_ = new uint8_t[cap]; }
  void Destroy() override { delete this; }
  uint32_t Capacity() const override { return cap_; }
  uint8_t* Data() override { return data_; }
  void SetSize(uint32_t s) override { size_ = s; }
  uint32_t Size() const override { return size_; }
 private:
  ~HeapBuffer() override { delete[] data_; }
  uint8_t* data_; uint32_t cap_; uint32_t size_ = 0;
};

class HeapBlock : public cdm::DecryptedBlock {
 public:
  void SetDecryptedBuffer(cdm::Buffer* b) override { buf_ = b; }
  cdm::Buffer* DecryptedBuffer() override { return buf_; }
  void SetTimestamp(int64_t t) override { ts_ = t; }
  int64_t Timestamp() const override { return ts_; }
 private:
  cdm::Buffer* buf_ = nullptr; int64_t ts_ = 0;
};

struct StoreFile : public cdm::FileIO {
  std::string dir; cdm::FileIOClient* client; std::string path;
  std::vector<char> read_buf;  // outlives OnReadComplete — the client may
  // retain the pointer past this call; a stack vector would dangle
  StoreFile(std::string d, cdm::FileIOClient* c) : dir(std::move(d)), client(c) {}
  void Open(const char* n, uint32_t len) override {
    path = dir + "/" + std::string(n, len);
    client->OnOpenComplete(cdm::FileIOClient::Status::kSuccess);
  }
  void Read() override {
    std::ifstream in(path, std::ios::binary);
    if (!in.good()) { client->OnReadComplete(cdm::FileIOClient::Status::kSuccess, nullptr, 0); return; }
    read_buf.assign(std::istreambuf_iterator<char>(in), {});
    client->OnReadComplete(cdm::FileIOClient::Status::kSuccess,
                           (const uint8_t*)read_buf.data(), read_buf.size());
  }
  void Write(const uint8_t* d, uint32_t n) override {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (d && n) out.write((const char*)d, n);
    client->OnWriteComplete(out.good() ? cdm::FileIOClient::Status::kSuccess
                                       : cdm::FileIOClient::Status::kError);
  }
  void Close() override { delete this; }
};

struct Msg { int type; std::string bytes; };

struct MelbaCdmImpl : public cdm::Host_10 {
  void* dl = nullptr;
  cdm::ContentDecryptionModule_10* cdm = nullptr;
  std::string store_dir;
  std::mutex mu; std::condition_variable cv;
  // timer lifecycle: destroy() flips alive, wakes every pending timer and
  // joins them; an in-flight TimerExpired holds a shared cdm_mu so the CDM
  // object can't be freed underneath it.
  std::mutex tmu; std::condition_variable tcv;
  std::shared_mutex cdm_mu;
  bool alive = true;
  std::vector<std::thread> timers;
  bool initialized = false;
  bool usable_key = false;
  std::map<uint32_t, int> promises;  // 0 pending, 1 resolved, -1 rejected
  std::map<uint32_t, std::string> session_of;
  std::string last_session;  // UpdateSession targets the newest session,
  std::deque<Msg> msgs;
  std::vector<uint8_t> content_kid;
  int reject_code = 0;

  cdm::Buffer* Allocate(uint32_t cap) override { return new HeapBuffer(cap); }
  void SetTimer(int64_t ms, void* ctx) override {
    std::lock_guard<std::mutex> g(tmu);
    if (!alive) return;
    timers.emplace_back(std::thread([ms, ctx, this] {
      {
        std::unique_lock<std::mutex> lk(tmu);
        if (tcv.wait_for(lk, std::chrono::milliseconds(ms),
                         [&] { return !alive; }))
          return;  // destroyed before the timer fired
      }
      std::shared_lock<std::shared_mutex> cg(cdm_mu);
      if (cdm && alive) cdm->TimerExpired(ctx);
    }));
  }
  cdm::Time GetCurrentWallTime() override {
    return std::chrono::duration<double>(
        std::chrono::system_clock::now().time_since_epoch()).count();
  }
  void OnInitialized(bool ok) override {
    { std::lock_guard<std::mutex> l(mu); initialized = ok; }
    cv.notify_all();
  }
  void OnResolveKeyStatusPromise(uint32_t, cdm::KeyStatus) override {}
  void OnResolveNewSessionPromise(uint32_t id, const char* sid, uint32_t n) override {
    { std::lock_guard<std::mutex> l(mu);
      promises[id] = 1; session_of[id].assign(sid, n);
      last_session.assign(sid, n); }
    cv.notify_all();
  }
  void OnResolvePromise(uint32_t id) override {
    { std::lock_guard<std::mutex> l(mu); promises[id] = 1; }
    cv.notify_all();
  }
  void OnRejectPromise(uint32_t id, cdm::Exception, uint32_t code,
                       const char*, uint32_t) override {
    { std::lock_guard<std::mutex> l(mu); promises[id] = -1; reject_code = code; }
    cv.notify_all();
  }
  void OnSessionMessage(const char*, uint32_t, cdm::MessageType t,
                        const char* m, uint32_t n) override {
    { std::lock_guard<std::mutex> l(mu);
      msgs.push_back({(int)t, std::string(m, n)}); }
    cv.notify_all();
  }
  void OnSessionKeysChange(const char*, uint32_t, bool,
                           const cdm::KeyInformation* keys, uint32_t n) override {
    { std::lock_guard<std::mutex> l(mu);
      for (uint32_t i = 0; i < n; i++)
        if (keys[i].status == cdm::kUsable) usable_key = true; }
    cv.notify_all();
  }
  void OnExpirationChange(const char*, uint32_t, cdm::Time) override {}
  void OnSessionClosed(const char*, uint32_t) override { cv.notify_all(); }
  void SendPlatformChallenge(const char*, uint32_t, const char*, uint32_t) override {}
  void EnableOutputProtection(uint32_t) override {}
  void QueryOutputProtectionStatus() override {}
  void OnDeferredInitializationDone(cdm::StreamType, cdm::Status) override {}
  cdm::FileIO* CreateFileIO(cdm::FileIOClient* c) override {
    return new StoreFile(store_dir, c);
  }
  void RequestStorageId(uint32_t v) override {
    static const uint8_t sid[32] = {'m','e','l','b','a','-','c','d','m'};
    if (cdm) cdm->OnStorageId(v, sid, sizeof(sid));
  }
};

static void* host_for(int, void* ud) { return ud; }
// Promise ids are process-global across every session: sessions on
// different threads must never hand the CDM the same id, or one
// session's resolve can answer another's wait.
std::atomic<uint32_t> next_promise{1};

}  // namespace

extern "C" {

MelbaCdm* melba_cdm_create(const char* so_path, const char* storage_dir) {
  auto* s = new MelbaCdmImpl();
  s->store_dir = storage_dir;
  s->dl = dlopen(so_path, RTLD_NOW);
  if (!s->dl) { delete s; return nullptr; }
  auto init = (void (*)())dlsym(s->dl, "InitializeCdmModule_4");
  auto create = (void* (*)(int, const char*, uint32_t,
                           void* (*)(int, void*), void*))dlsym(s->dl, "CreateCdmInstance");
  if (!init || !create) { dlclose(s->dl); delete s; return nullptr; }
  init();
  const char* ks = "com.widevine.alpha";
  void* raw = create(10, ks, strlen(ks), host_for, s);
  if (!raw) { dlclose(s->dl); delete s; return nullptr; }
  s->cdm = static_cast<cdm::ContentDecryptionModule_10*>(raw);
  s->cdm->Initialize(false, true, false);
  return reinterpret_cast<MelbaCdm*>(s);
}

int melba_cdm_init(MelbaCdm* h, int timeout_ms) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  std::unique_lock<std::mutex> l(s->mu);
  if (!s->cv.wait_for(l, std::chrono::milliseconds(timeout_ms),
                      [&]{ return s->initialized; })) return -1;
  return 0;
}

static int wait_promise(MelbaCdmImpl* s, uint32_t id, int ms = 15000) {
  std::unique_lock<std::mutex> l(s->mu);
  if (!s->cv.wait_for(l, std::chrono::milliseconds(ms),
                      [&]{ return s->promises[id] != 0; })) return -99;
  return s->promises[id];
}

int melba_cdm_set_certificate(MelbaCdm* h, const uint8_t* c, uint32_t len) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  uint32_t id = next_promise++;
  { std::lock_guard<std::mutex> l(s->mu); s->promises[id] = 0; }
  s->cdm->SetServerCertificate(id, c, len);
  int r = wait_promise(s, id);
  return r == 1 ? 0 : -2;
}

int melba_cdm_request(MelbaCdm* h, const uint8_t* init, uint32_t len) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  uint32_t id = next_promise++;
  { std::lock_guard<std::mutex> l(s->mu); s->promises[id] = 0; }
  s->cdm->CreateSessionAndGenerateRequest(id, cdm::kTemporary, cdm::kCenc,
                                          init, len);
  int r = wait_promise(s, id);
  return r == 1 ? (int)id : (r == -1 ? -2 : -99);
}

int melba_cdm_next_message(MelbaCdm* h, uint8_t* buf, uint32_t cap,
                         int* type, int timeout_ms) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  std::unique_lock<std::mutex> l(s->mu);
  if (!s->cv.wait_for(l, std::chrono::milliseconds(timeout_ms),
                      [&]{ return !s->msgs.empty(); })) return 0;
  const Msg& m = s->msgs.front();
  *type = m.type;
  // peek, don't pop: an oversized message must stay queued so the
  // caller's "retry with a bigger buffer" actually delivers it — popping
  // here silently drops the license request and the session stalls.
  if (m.bytes.size() > cap) return (int)m.bytes.size();
  memcpy(buf, m.bytes.data(), m.bytes.size());
  int n = (int)m.bytes.size();
  s->msgs.pop_front();
  return n;
}

int melba_cdm_update(MelbaCdm* h, const uint8_t* lic, uint32_t len) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  std::string sid;
  { std::lock_guard<std::mutex> l(s->mu); sid = s->last_session; }
  if (sid.empty()) return -3;
  uint32_t id = next_promise++;
  { std::lock_guard<std::mutex> l(s->mu); s->promises[id] = 0; }
  s->cdm->UpdateSession(id, sid.data(), sid.size(), lic, len);
  int r = wait_promise(s, id);
  return r == 1 ? 0 : -2;
}

void melba_cdm_set_kid(MelbaCdm* h, const uint8_t* kid, uint32_t len) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  s->content_kid.assign(kid, kid + len);
}

int melba_cdm_has_usable_key(MelbaCdm* h) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  std::lock_guard<std::mutex> l(s->mu);
  return s->usable_key ? 1 : 0;
}

int melba_cdm_decrypt(MelbaCdm* h, uint32_t scheme, const uint8_t* iv, uint32_t ivlen,
                    const uint32_t* pairs, uint32_t nsubs,
                    const uint8_t* data, uint32_t dlen,
                    uint8_t* out, uint32_t outcap) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  std::vector<cdm::SubsampleEntry> subs(nsubs);
  for (uint32_t i = 0; i < nsubs; i++) {
    subs[i].clear_bytes = pairs[i * 2];
    subs[i].cipher_bytes = pairs[i * 2 + 1];
  }
  cdm::InputBuffer_2 in = {};
  in.data = data; in.data_size = dlen;
  in.encryption_scheme = (cdm::EncryptionScheme)scheme;
  in.key_id = s->content_kid.data(); in.key_id_size = s->content_kid.size();
  in.iv = iv; in.iv_size = ivlen;
  in.subsamples = subs.data(); in.num_subsamples = nsubs;
  in.timestamp = 0;
  HeapBlock blk;
  auto st = s->cdm->Decrypt(in, &blk);
  if (st != cdm::kSuccess || !blk.DecryptedBuffer()) return -(int)st - 1;
  auto* b = blk.DecryptedBuffer();
  if (b->Size() > outcap) { b->Destroy(); return -100; }
  memcpy(out, b->Data(), b->Size());
  uint32_t n = b->Size();
  b->Destroy();
  return (int)n;
}

void melba_cdm_destroy(MelbaCdm* h) {
  auto* s = reinterpret_cast<MelbaCdmImpl*>(h);
  {
    std::lock_guard<std::mutex> l(s->tmu);
    s->alive = false;
  }
  s->tcv.notify_all();
  // timers wake on !alive and exit without touching cdm; an in-flight
  // TimerExpired holds shared cdm_mu and finishes before we lock it.
  std::vector<std::thread> ts;
  {
    std::lock_guard<std::mutex> l(s->tmu);
    ts = std::move(s->timers);
  }
  for (auto& t : ts) if (t.joinable()) t.join();
  {
    std::unique_lock<std::shared_mutex> g(s->cdm_mu);
    if (s->cdm) { s->cdm->Destroy(); s->cdm = nullptr; }
  }
  // Never dlclose here: Destroy() does not synchronously join every CDM
  // internal thread, and unmapping the .so while one still runs segfaults
  // inside libwidevinecdm (measured 2026-10-22, Vivaldi CDM 4.10.3112.0).
  // The handle stays mapped for the process lifetime, same as Chrome.
  delete s;
}

}  // extern "C"
