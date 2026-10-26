# Widevine CDM host experiment — aml full playback

Goal: aml hosts the user's licensed libwidevinecdm.so and plays Apple
Music streams natively. NOT circumvention: keys never leave the CDM;
aml implements the public CDM host interface like a browser/Kodi does.

CDM found on host:
  ~/.config/vivaldi/WidevineCdm/4.10.3112.0/_platform_specific/linux_x64/libwidevinecdm.so
  (CDM 4.10.3112.0; L3 software CDM, same as Linux Chrome web player)

## Experiments (ascending)

- P-CDM-0  dlopen + InitializeCdmModule + CreateCdmInstance via thin C++
           shim (vtable interop is risky in pure Rust — C++ shim exposes
           flat C API to Rust, same shape as Kodi wvdecrypter)
- P-CDM-1  AAC lossy track: cenc PSSH -> CreateSessionAndGenerateRequest
           -> OnSessionMessage -> POST license body to Apple license URL
           -> UpdateSession -> OnSessionKeysChange -> DecryptAndDecode
           -> PCM. Manifest+license URLs come from MusicKit playback
           request flow (authenticated API calls with media-user-token;
           verify endpoints against a live user session or public docs).
- P-CDM-2  Same CDM, Android-profile playback request -> does the
           license server issue ALAC keys to a desktop CDM identity?
           YES => legitimate lossless. NO => client-type binding;
           lossless legit path = real Android runtime (Waydroid).

## Honest boundaries

- Requires a valid media-user-token (aml auth sync).
- Verdict label: STREAMING/DECODED, never BITPERFECT.
- ToS gray (non-approved client) but clean re: anti-circumvention:
  licensed CDM, no key extraction, subscription token.
- License acquisition details (endpoints/params) still unverified —
  replicate from public documentation or authorized live capture.

## P-CDM-0 RESULT (2026-10-15): PASS

dlopen + InitializeCdmModule_4 + CreateCdmInstance(v10, com.widevine.alpha)
+ Initialize() -> OnInitialized(1) on Vivaldi's CDM 4.10.3112.0.
The CDM does not demand a Chrome sandbox identity to initialize.
Probe: crates/aml-cdm/shim/probe.cc (C++ host stub, all Host_10 virtuals).
Headers vendored from chromium/cdm (BSD) at crates/aml-cdm/third_party/cdm-api/.

Next: P-CDM-1 needs (a) media-user-token via `aml auth sync` after the
user signs into music.apple.com, and (b) Apple playback manifest +
license endpoint details (from authorized live capture or public docs).
