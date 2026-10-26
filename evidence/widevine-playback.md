# Apple Music webPlayback via Widevine CDM — verified end-to-end

Date: 2026-10-17. Test track: YOASOBI "夜に駆ける" (adamId 1490256995).

## Working pipeline (all steps verified at runtime)

1. `POST MZPlay.woa/wa/webPlayback {"salableAdamId": id}` — headers
   `Authorization: Bearer <dev>`, `media-user-token`, `X-Apple-Store-Front`.
   Returns `songList[0].assets` — pick a `ctrp*` flavor (CENC/Widevine;
   `cbcp*` = FairPlay, `ibhp*` = legacy HLS). `hls-key-server-url` and
   `widevine-cert-url` come from the same object.
2. Asset `URL` is an fMP4 HLS playlist. `EXT-X-KEY` for ctrp carries
   `URI="data:;base64,<16-byte KID>"`; `EXT-X-MAP` gives init range and
   each `EXT-X-BYTERANGE` is one moof+mdat fragment of a single mp4.
3. CDM session (libwidevinecdm.so 4.10.3112.0 via public Host_10):
   - must answer `RequestStorageId` with `OnStorageId` (32-byte id) or
     the session stalls
   - `SetServerCertificate(<widevine-cert-url body>, 705B)` resolves
   - initData = the exact pssh MusicKit generates for a bare KID
     (v0 box, data = protobuf `08 01 12 10 <KID>`), NOT a v1 KID-list
4. `CreateSessionAndGenerateRequest(kTemporary, kCenc, pssh)` →
   license-request challenge (~1.7–2.1 KB).
5. `POST acquireWebPlaybackLicense` with MusicKit's body
   `{challenge:b64, uri:"data:;base64,<kid>", key-system, adamId,
   isLibrary:false, user-initiated:true}` and headers
   `X-Apple-Music-User-Token` (NOT `media-user-token`),
   `X-Apple-Renewal:true`. No `/start` suffix for this path.
   Response `{license:b64, renew-after:780, status:0}`.
6. `UpdateSession(license)` → `OnSessionKeysChange` reports the content
   KID `status=kUsable`.
7. Per sample: `Decrypt` with `encryption_scheme=kCenc`, per-sample IV
   and subsamples parsed from `senc`. Verified 430/430 samples across
   fragments; output frames decode as AAC-LC 44.1 kHz stereo (ADTS
   wrapped → ffprobe/ffmpeg clean, ~10 s per fragment).

License errors observed: `{"errorCode":-42585,"status":-1021}` when the
challenge pssh was malformed (v1 KID-list) or when the user token was
sent under the wrong header name.

## Lossless boundary — REVISED (ALAC proven end-to-end for segment 0)

The earlier "lossless not offered" conclusion was wrong in detail.
The catalog API exposes lossless directly:

- `GET /v1/catalog/jp/songs/{id}?extend=extendedAssetUrls` returns
  `attributes.extendedAssetUrls.enhancedHls` — a lossless HLS master
  m3u8, free of charge, no extra entitlement request.
- The master advertises `CODECS="alac"` (~1411 kbps, 44.1 kHz/16-bit
  stereo) plus AAC/HE-AAC variants, `EXT-X-SESSION-DATA` blocks
  `com.apple.hls.AudioSessionKeyInfo` (per-key FairPlay/PlayReady/
  Widevine init material) and `com.apple.hls.audioAssetMetadata`
  (variant → `AUDIO-SESSION-KEY-IDS` map).
- Each session key exists in three formats: `skd://` (FairPlay),
  PlayReady WRM data URI, and a Widevine pssh data URI
  (`urn:uuid:edef8ba9-…`, 56-byte v0 pssh with content-id, not a KID).
- Key layout per variant: key "1" = `skd://…/P000000000/s1/e1`
  (generic, covers the FIRST media segment of every variant);
  key "3" = `skd://…/p1479352138/c23` for ALAC segments 2..17;
  key "2" = `…/c22` for AAC variants.

### What is verified working (segment 0, runtime-verified audio)

- License the key-"1" session pssh via `acquireWebPlaybackLicense`
  (same body/headers as the AAC path) → status 0, usable key
  `000000000000000073312f6531202020` ("s1/e1").
- ALAC media is `cbcs` (not cenc): `tenc` v1, `Per_Sample_IV_Size=0`,
  a 16-byte **constant IV** carried in `tenc`, `senc` entries carry
  subsample pairs only (u16 nsub + {u16 clear, u32 cipher}).
- `Decrypt(scheme=kCbcs, iv=const-IV)` on the s1/e1 session →
  **161/161 samples decrypted** from the first 1.57 MB segment.
- Rebuilt init (enca→alac, sinf stripped) + moof + decrypted mdat
  feeds ffprobe: `alac`, 44100 Hz, 2 ch, s16p, duration 14.9537 s
  (matches playlist FIRST-SEGMENT-DURATION). Decoded PCM is real
  music (peak ≈0 dBFS, RMS −13.5 dB).

### What is verified NOT working (server-side gate)

`acquireWebPlaybackLicense` refuses the c23 session key for this web
session in every tested configuration — status `-1002`:
- challenges from the desktop CDM (Vivaldi) and from a public Android
  emulator WV device blob (gamdl's `wvd.py`) alike
- `uri` = WV data URI, `skd://` URI, or compound
- `adamId` ∈ {1490256995, p1479352138, P1479352138, 1479352138, c23}
- bodies: plain, `license-requests` (HTTP 500), `streaming-keys`
  (HTTP 500), extra store fields (salableAdamId/pricing/productType)
- license_type STREAMING / OFFLINE
- endpoints: `acquireWebPlaybackLicense` (+`/start`, GET) → 404/405;
  `acquireLicense` on several host/path guesses → 404
Contrast: the AAC sibling key c22 answered `status:0` with `uri`=skd
URI — the service parses these requests and decides per-key-class;
lossless keys are simply not issued to this client class. This agrees
with every public implementation (gamdl, onthespot, ramdl): identical
WV flow for AAC, ALAC routed to an Android-library emulator
("wrapper") that runs Apple's own FairPlay client code.

### Remaining routes to full-song ALAC (not yet implemented)

1. Android-libs "wrapper": Apple Music's own native libs
   (`libCoreLSKD` etc., x86_64 Android build — runs under a bionic
   chroot, no CPU emulation needed; `wrapper-lite`/`wrapper-v2` are
   the reference launchers). Requires the app's .so files (not
   redistributable; user supplies).
2. PlayReady session key: needs a real PR device cert
   (`bgroupcert.dat`+`zgpriv.dat` — not publicly distributable), and
   whether Apple licenses c23 to PR is unknown.
3. FairPlay SPC in pure software: SPC is generated by Apple's closed
   client stack — no public generator; equivalent effort to route 1.

Evidence files (crates/aml-cdm/target/probe_work/):
`alac_init.mp4` (1023 B), `alac_frag0.mp4` (first segment, cbcs),
`out.bin` (161 decrypted samples), `probe_k1.log` (KEYS_USABLE=1).

## Renewal

Licenses carry `renew-after` (~780 s). MusicKit renewal sequence:
`UpdateSession("renew")` → CDM emits a license-renewal message (type 1)
→ same license POST → `UpdateSession(response)`. `CdmSession` exposes
`license_renewal()` for that message; per-track sessions normally end
inside the window.

## Files

- `crates/aml-cdm/shim/cdm_shim.cc` + `aml_cdm.h` — Host_10 impl + C ABI
- `crates/aml-cdm/src/lib.rs` — CdmSession, WebPlayback, fMP4 parser
- `crates/aml-cdm/src/bin/e2e.rs` — full-pipeline verifier
- `crates/aml-cdm/shim/license_probe.cc` — original file-handoff probe

## Productization status (player integration)

- `crates/aml-cdm/src/stream.rs` — virtual decrypted fMP4:
  `VirtualMedia` = `[clean_init][frag0][frag1]…` byte-level
  `Read + Seek` source; lazy per-segment fetch+decrypt with a
  4-ahead parallel prefetch (raw bytes fetched on background
  threads; decryption stays serialized on the CDM/session side).
- Init rewrite: `enca` entries become their `frma` type (`alac`/`mp4a`),
  `sinf` stripped, sizes recomputed, stsd collapsed to ONE entry —
  symphonia rejects multi-entry stsd outright; fragments' crypto
  entry is still chosen correctly via `tfhd.sample_description_index`
  (per-entry `EntryCrypto` kept in `InitInfo`).
- Apple `tenc` emits two reserved bytes before `isProtected` in both
  v0 and v1 — handled by a sanity-probing parser.
- AAC runtime verification: `aml ctl open stream:1490256995` →
  MPRIS `PlaybackStatus=Playing`, position advancing (≈4.7 s → ≈16.7 s),
  `ctl setpos 60` lands ≈64 s. Full 4:21 track = 27 fMP4 segments
  fetched+decrypted+decoded through symphonia AAC → ALSA.
- CDM shim timer use-after-free fixed: timer threads are tracked,
  woken and joined before CDM destroy (was a segfault on teardown).
- `Dec` backends: `Cdm` (in-CDM), `Aes` (local AES-128-CBC cbcs with
  content keys from a key service; the well-known `s1/e1` prefetch
  content key is compiled in), `Wrapper` (wrapper-v2 TCP protocol:
  `WV2D` magic, v1, BATCH frames carrying adam_id+skd_uri+aligned
  cipher blocks → plaintext; same wire format as gamdl's ammuxer).
- `open_alac` picks the backend by `wrapper_url` scheme:
  `tcp://host:port`/bare `host:port` → Wrapper; `http(s)://` →
  key-extraction HTTP service (`GET ?adamId&uri` → `contentKey`).
- Unit tests: playlist/master parsing, key rotation, stsd rewrite
  (2 enca → 1 alac, size fixups), tfhd desc-index, AES-CBC cbcs
  roundtrip, wrapper-v2 frame roundtrip against a mock TCP server.
  `cargo test -p aml-cdm`: 6/6 green.
- Token freshness: `auth open` now validates captured
  `media-user-token` against `/v1/me/storefront` before storing —
  stale cookies no longer report success.

## Productization pass (post-integration)

- **cbcs chaining fixed**: a sample's cipher subsample regions form ONE
  CBC stream — IV chains across region boundaries and 16B alignment
  applies to the concatenation (verified against gamdl's
  `cbcs_ciphertext_for_sample`/`reassemble_sample` semantics). The prior
  per-region IV reset would corrupt multi-region samples. Unit tests now
  cover a 20+20B two-region sample and crypt/skip pattern windows
  (`tenc` v1 fields are parsed; the wrapper path rejects nonzero
  patterns since the service expects whole-region cipher blobs, same as
  gamdl).
- **wrapper-v2 parity**: request/response framing re-verified byte for
  byte against gamdl's ammuxer client (u16 adam/skd lengths, u32 count +
  length table + labels + aligned blobs; response strict — count match,
  no trailing bytes). Splice asserts decrypted length equals the enc
  total instead of clamping silently.
- **License renewal**: `Dec::Cdm` carries `RenewCtx`; ~60s before the
  license's `renew-after` it runs MusicKit's sequence
  (`update("renew")` → type-1 message → same license POST → `update`)
  on the load path, so >13min tracks keep playing.
- **UI wiring**: catalog play/enqueue/play-all/deeplink send `stream:`
  when a token is stored, `preview:` otherwise; `open_uri` no longer
  wraps `stream:` in `file://` (was a hard bug). Daemon falls back to
  the same song's public preview when `stream:` open fails (expired
  token, no wrapper) — verdict carries the reason, no silent skip.
- **auth status** now probes `/v1/me/storefront` live; a stored-but-
  dead token reports INVALID with a re-login hint instead of a stale
  positive exp.
- Workspace `default-members` covers `aml-cdm`: bare `cargo fmt/
  clippy/test` (what CI runs) includes it.
- Gates: `cargo fmt --check`, `clippy --all-targets -D warnings`,
  `cargo test` (11 suites, incl. 8 aml-cdm) all green.

### Still gated on a fresh login

The stored token currently returns 401 (server-side session ended).
Live re-verification of AAC playback and the full-ALAC path needs
`aml auth open` once, plus a wrapper-v2 service for track-specific
FairPlay keys.
