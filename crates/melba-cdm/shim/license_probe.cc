// license_probe: drives libwidevinecdm.so through the Apple Music
// webPlayback license exchange. Reads:
//   <workdir>/widevine.bin   widevineCert endpoint body
//   <workdir>/kid.bin        16-byte content key id
// Writes:
//   <workdir>/challenge.bin  CDM license request
// Then waits for <workdir>/license.bin and calls UpdateSession.
#include <dlfcn.h>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <map>
#include <string>
#include <thread>
#include <vector>
#include "content_decryption_module.h"

static std::string g_store;
static cdm::ContentDecryptionModule_10* g_cdm = nullptr;

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

class StoreFile : public cdm::FileIO {
 public:
  explicit StoreFile(cdm::FileIOClient* client) : client_(client) {}
  void Open(const char* name, uint32_t size) override {
    path_ = g_store + "/" + std::string(name, size);
    std::ifstream in(path_, std::ios::binary);
    exists_ = in.good();
    client_->OnOpenComplete(cdm::FileIOClient::Status::kSuccess);
  }
  void Read() override {
    std::ifstream in(path_, std::ios::binary);
    std::vector<char> d((std::istreambuf_iterator<char>(in)), {});
    if (!exists_ || !in.good())
      client_->OnReadComplete(cdm::FileIOClient::Status::kSuccess, nullptr, 0);
    else
      client_->OnReadComplete(cdm::FileIOClient::Status::kSuccess,
                              (const uint8_t*)d.data(), d.size());
  }
  void Write(const uint8_t* d, uint32_t n) override {
    std::ofstream out(path_, std::ios::binary | std::ios::trunc);
    if (d && n) out.write((const char*)d, n);
    client_->OnWriteComplete(out.good() ? cdm::FileIOClient::Status::kSuccess
                                        : cdm::FileIOClient::Status::kError);
  }
  void Close() override { delete this; }
 private:
  cdm::FileIOClient* client_; std::string path_; bool exists_ = false;
};

class LicenseHost : public cdm::Host_10 {
 public:
  enum Res { kPending, kResolved, kRejected };
  std::map<uint32_t, Res> promises;
  std::map<uint32_t, std::string> session_of_promise;
  std::string workdir;
  bool got_challenge = false, keys_usable = false;

  cdm::Buffer* Allocate(uint32_t cap) override { return new HeapBuffer(cap); }
  void SetTimer(int64_t ms, void* ctx) override {
    std::thread([ms, ctx] {
      std::this_thread::sleep_for(std::chrono::milliseconds(ms));
      if (g_cdm) g_cdm->TimerExpired(ctx);
    }).detach();
  }
  cdm::Time GetCurrentWallTime() override {
    return std::chrono::duration<double>(
        std::chrono::system_clock::now().time_since_epoch()).count();
  }
  void OnInitialized(bool ok) override { printf("[host] OnInitialized=%d\n", ok); }
  void OnResolveKeyStatusPromise(uint32_t, cdm::KeyStatus) override {}
  void OnResolveNewSessionPromise(uint32_t id, const char* sid,
                                  uint32_t n) override {
    promises[id] = kResolved;
    session_of_promise[id].assign(sid, n);
    printf("[host] session established (%u bytes)\n", n);
  }
  void OnResolvePromise(uint32_t id) override {
    promises[id] = kResolved;
    printf("[host] promise %u resolved\n", id);
  }
  void OnRejectPromise(uint32_t id, cdm::Exception e, uint32_t code,
                       const char* msg, uint32_t n) override {
    promises[id] = kRejected;
    printf("[host] promise %u REJECTED exc=%u code=%u msg=%.*s\n", id, e, code,
           n, msg ? msg : "");
  }
  void OnSessionMessage(const char* sid, uint32_t sids, cdm::MessageType t,
                        const char* msg, uint32_t msgs) override {
    printf("[host] OnSessionMessage type=%d size=%u\n", t, msgs);
    if (t == cdm::kLicenseRequest || t == cdm::kLicenseRenewal) {
      std::ofstream out(workdir + "/challenge.bin", std::ios::binary);
      out.write(msg, msgs);
      got_challenge = true;
      printf("[host] CHALLENGE_READY %s/challenge.bin\n", workdir.c_str());
    }
  }
  void OnSessionKeysChange(const char* sid, uint32_t sids, bool added,
                           const cdm::KeyInformation* keys,
                           uint32_t nkeys) override {
    printf("[host] OnSessionKeysChange added=%d keys=%u:", added, nkeys);
    for (uint32_t i = 0; i < nkeys; i++) {
      printf(" [");
      for (uint32_t j = 0; j < keys[i].key_id_size && j < 16; j++)
        printf("%02x", keys[i].key_id[j]);
      printf(" status=%u]", keys[i].status);
      if (keys[i].status == cdm::kUsable) keys_usable = true;
    }
    printf("\n");
  }
  void OnExpirationChange(const char*, uint32_t, cdm::Time) override {}
  void OnSessionClosed(const char*, uint32_t) override {
    printf("[host] OnSessionClosed\n");
  }
  void SendPlatformChallenge(const char*, uint32_t, const char* c,
                             uint32_t n) override {
    printf("[host] SendPlatformChallenge size=%u (ignored)\n", n);
  }
  void EnableOutputProtection(uint32_t) override {}
  void QueryOutputProtectionStatus() override {}
  void OnDeferredInitializationDone(cdm::StreamType, cdm::Status) override {}
  cdm::FileIO* CreateFileIO(cdm::FileIOClient* c) override {
    return new StoreFile(c);
  }
  void RequestStorageId(uint32_t v) override {
    printf("[host] RequestStorageId v=%u -> providing fixed 32B id\n", v);
    static const uint8_t sid[32] = {0x61,0x6d,0x6c};
    if (g_cdm) g_cdm->OnStorageId(v, sid, 32);
  }
};

static void* g_host = nullptr;
static void* get_cdm_host(int, void*) { return g_host; }

static std::vector<uint8_t> read_file(const std::string& p) {
  std::ifstream in(p, std::ios::binary);
  return {std::istreambuf_iterator<char>(in), {}};
}

int main(int argc, char** argv) {
  setbuf(stdout, nullptr);
  if (argc < 3) { fprintf(stderr, "usage: %s <cdm.so> <workdir>\n", argv[0]); return 2; }
  const std::string workdir = argv[2];
  g_store = workdir + "/cdmstore";

  void* h = dlopen(argv[1], RTLD_NOW);
  if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
  auto ver = (const char* (*)())dlsym(h, "GetCdmVersion");
  auto init = (void (*)())dlsym(h, "InitializeCdmModule_4");
  auto create = (void* (*)(int, const char*, uint32_t,
                           void* (*)(int, void*), void*))dlsym(h, "CreateCdmInstance");
  printf("GetCdmVersion: %s\n", ver ? ver() : "?");
  if (init) init();

  LicenseHost host; host.workdir = workdir; g_host = &host;
  const char* ks = "com.widevine.alpha";
  void* raw = create(10, ks, strlen(ks), get_cdm_host, nullptr);
  if (!raw) { printf("CreateCdmInstance failed\n"); return 1; }
  auto* cdm = static_cast<cdm::ContentDecryptionModule_10*>(raw);
  g_cdm = cdm;
  cdm->Initialize(false, true, false);
  std::this_thread::sleep_for(std::chrono::milliseconds(300));

  auto cert = read_file(workdir + "/widevine.bin");
  if (cert.empty()) { printf("no cert\n"); return 1; }
  printf("-> SetServerCertificate (%zu bytes)\n", cert.size());
  cdm->SetServerCertificate(1, cert.data(), cert.size());
  for (int i = 0; i < 100 && host.promises[1] == LicenseHost::kPending; i++)
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
  printf("   cert result: %s\n",
         host.promises[1] == LicenseHost::kResolved ? "resolved"
         : host.promises[1] == LicenseHost::kRejected ? "rejected" : "timeout");

  // verbatim pssh if provided (e.g. ALAC session-key pssh), else synthesize
  std::vector<uint8_t> pssh = read_file(workdir + "/pssh.bin");
  std::vector<uint8_t> kid;
  if (!pssh.empty()) {
    printf("-> using verbatim pssh (%zu bytes)\n", pssh.size());
    // kid for decrypt = last 22 bytes hold "12 10 <16B>"
    for (size_t i = 4; i + 20 <= pssh.size(); i++)
      if (pssh[i] == 0x12 && pssh[i+1] == 0x10)
        kid.assign(pssh.begin()+i+2, pssh.begin()+i+18);
    if (kid.empty()) kid.assign(16, 0);
  } else {
  auto kid0 = read_file(workdir + "/kid.bin");
  if (kid0.size() != 16) { printf("bad kid size %zu\n", kid0.size()); return 1; }
  kid = kid0;
  // [52]['pssh'][v0][wv-sysid][datasize=20][0x08 0x01 0x12 0x10][kid16]
  static const uint8_t wv_sysid[16] = {0xed,0xef,0x8b,0xa9,0x79,0xd6,0x4a,0xce,
                                       0xa3,0xc8,0x27,0xdc,0xd5,0x1d,0x21,0xed};
  auto be32 = [&](uint32_t v){ for (int s=24;s>=0;s-=8) pssh.push_back(v>>s); };
  be32(52);
  pssh.insert(pssh.end(), {'p','s','s','h'});
  be32(0);
  pssh.insert(pssh.end(), wv_sysid, wv_sysid+16);
  be32(20);
  pssh.insert(pssh.end(), {0x08,0x01,0x12,0x10});
  pssh.insert(pssh.end(), kid.begin(), kid.end());
  }
  printf("-> CreateSessionAndGenerateRequest (pssh %zu bytes)\n", pssh.size());
  cdm->CreateSessionAndGenerateRequest(2, cdm::kTemporary, cdm::kCenc,
                                       pssh.data(), pssh.size());
  for (int i = 0; i < 300 && !host.got_challenge
                       && host.promises[2] != LicenseHost::kRejected; i++)
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
  if (!host.got_challenge) { printf("no challenge produced\n"); return 1; }

  printf("-> waiting for license.bin ...\n");
  std::vector<uint8_t> lic;
  for (int i = 0; i < 1800; i++) {
    lic = read_file(workdir + "/license.bin");
    if (!lic.empty()) break;
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
  }
  if (lic.empty()) { printf("license.bin never appeared\n"); return 1; }

  std::string sid = host.session_of_promise[2];
  printf("-> UpdateSession (%zu bytes)\n", lic.size());
  cdm->UpdateSession(3, sid.data(), sid.size(), lic.data(), lic.size());
  for (int i = 0; i < 100 && host.promises[3] == LicenseHost::kPending; i++)
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
  printf("   update result: %s\n",
         host.promises[3] == LicenseHost::kResolved ? "resolved"
         : host.promises[3] == LicenseHost::kRejected ? "rejected" : "timeout");
  printf("KEYS_USABLE=%d\n", host.keys_usable ? 1 : 0);
  if (!host.keys_usable) { g_cdm = nullptr; cdm->Destroy(); return 1; }

  // decrypt phase: job.bin = LE [u32 iv_len][iv][u32 scheme][u32 ns]
  //                [ns x (u32 clear,u32 cipher)][u32 data_len][data]
  printf("-> waiting for job.bin (decrypt work) ...\n");
  std::vector<uint8_t> job;
  for (int i = 0; i < 1200; i++) {
    job = read_file(workdir + "/job.bin");
    if (!job.empty()) break;
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
  }
  if (job.empty()) { printf("no decrypt job; license path verified\n"); return 0; }

  auto rd32 = [&](size_t& o) -> uint32_t {
    if (o + 4 > job.size()) { o = job.size(); return 0; }
    uint32_t v = job[o]|(job[o+1]<<8)|(job[o+2]<<16)|((uint32_t)job[o+3]<<24);
    o += 4; return v;
  };
  auto rdbytes = [&](size_t& o, uint32_t n) -> const uint8_t* {
    if (n > job.size() - o) { o = job.size(); return nullptr; }
    const uint8_t* p = job.data() + o; o += n; return p;
  };
  size_t o = 0;
  uint32_t scheme = rd32(o);
  uint32_t count = rd32(o);
  printf("   job: scheme=%u samples=%u\n", scheme, count);

  std::ofstream wf(workdir + "/out.bin", std::ios::binary | std::ios::trunc);
  uint32_t ok = 0;
  for (uint32_t k = 0; k < count; k++) {
    uint32_t iv_len = rd32(o);
    const uint8_t* iv = rdbytes(o, iv_len);
    uint32_t ns = rd32(o);
    if (ns > (1u << 20)) { printf("   sample %u absurd ns=%u\n", k, ns); break; }
    std::vector<cdm::SubsampleEntry> subs(ns);
    for (auto& s : subs) { s.clear_bytes = rd32(o); s.cipher_bytes = rd32(o); }
    uint32_t data_len = rd32(o);
    const uint8_t* data = rdbytes(o, data_len);
    if (!iv || !data) { printf("   sample %u truncated job\n", k); break; }

    cdm::InputBuffer_2 in = {};
    in.data = data; in.data_size = data_len;
    in.encryption_scheme = (cdm::EncryptionScheme)scheme;
    in.key_id = kid.data(); in.key_id_size = kid.size();
    in.iv = iv; in.iv_size = iv_len;
    in.subsamples = subs.data(); in.num_subsamples = ns;
    in.timestamp = 0;

    HeapBlock out;
    auto st = cdm->Decrypt(in, &out);
    if (st != cdm::kSuccess || !out.DecryptedBuffer()) {
      printf("   sample %u Decrypt status=%u FAIL\n", k, st);
      continue;
    }
    wf.write((const char*)out.DecryptedBuffer()->Data(),
             out.DecryptedBuffer()->Size());
    ok++;
  }
  wf.close();
  printf("   DECRYPT_OK %u/%u samples -> %s/out.bin\n", ok, count,
         workdir.c_str());
  g_cdm = nullptr;  // detached timer threads must not touch a dead cdm
  cdm->Destroy();
  return ok == count ? 0 : 1;
}
