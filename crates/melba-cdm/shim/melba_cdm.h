#ifndef MELBA_CDM_H
#define MELBA_CDM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MelbaCdm MelbaCdm;

// Open the CDM .so and create an instance. storage_dir must exist.
MelbaCdm* melba_cdm_create(const char* so_path, const char* storage_dir);
// Wait for the CDM to report initialized. 0 ok, <0 error/timeout.
int melba_cdm_init(MelbaCdm*, int timeout_ms);
// Provide the Widevine service certificate. 0 resolved, <0 rejected.
int melba_cdm_set_certificate(MelbaCdm*, const uint8_t* cert, uint32_t len);
// Create a temporary session and start a license request for init_data
// (a 'cenc' pssh box). 0 ok, <0 rejected.
int melba_cdm_request(MelbaCdm*, const uint8_t* init_data, uint32_t len);
// Copy the next queued session message (license request / renewal).
// Returns bytes written (may exceed cap -> call again with bigger buf),
// 0 when no message within timeout_ms, <0 on closed session.
// *type receives the MessageType (0=request, 1=renewal, 2=release, 3=individualization).
int melba_cdm_next_message(MelbaCdm*, uint8_t* buf, uint32_t cap,
                         int* type, int timeout_ms);
// Feed a license response. 0 resolved, <0 rejected.
int melba_cdm_update(MelbaCdm*, const uint8_t* lic, uint32_t len);
// 1 if a usable key exists for key_id (16 bytes), else 0.
int melba_cdm_has_usable_key(MelbaCdm*);
// Set the content key id used for Decrypt calls.
void melba_cdm_set_kid(MelbaCdm*, const uint8_t* kid, uint32_t len);
// Decrypt one sample. subs = interleaved [clear,cipher] u32 pairs.
// Returns decrypted size, or -(status+1) on failure.
int melba_cdm_decrypt(MelbaCdm*, uint32_t scheme, const uint8_t* iv, uint32_t ivlen,
                    const uint32_t* subs_pairs, uint32_t nsubs,
                    const uint8_t* data, uint32_t dlen,
                    uint8_t* out, uint32_t outcap);
void melba_cdm_destroy(MelbaCdm*);

#ifdef __cplusplus
}
#endif
#endif
