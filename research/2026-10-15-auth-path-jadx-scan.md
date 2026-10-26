# Apple Music Android — auth/playback path analysis (jadx + native strings)

Companion to `2026-10-15-apple-music-linux-route-measurements.md` (same APK,
same sha256). This pass answers the open question that doc deferred: *where does
sign-in actually live, and could a native client follow the same path?*

Method: `jadx 1.5.6` full decompile (`~/am-jadx/out`, 13,270 java files) plus
`strings` over `lib/arm64-v8a/*.so` (`evidence/native-scan/*.endpoints.txt` —
those files are the raw grep output this doc interprets; marked UNVERIFIED in
the sibling doc until now). Read-only analysis; no execution, no credentials.

## The auth path, end to end (decompiled)

1. **All endpoint URLs are server-delivered.** Every authenticated surface —
   sign-in webview, commerce pages, token endpoints — resolves through the
   runtime `Bag` fetched from `init.itunes.apple.com/bag.xml` (string present in
   `libstoreservicescore.so`; sandbox twin `sandbox.itunes.apple.com/bag.xml`
   for Apple's internal store). `WebViewModel.getUrlFromParams()` literally does
   `url.replace("https://music.apple.com/", bag.urlFromKey("commerceAppBaseUrl"))`.
   There are **no static auth URLs in the app** — `idmsa` appears zero times in
   dex *and* in all native libs. Endpoint mining of the binary is a dead end by
   design.

2. **Sign-in is a hosted webview with a JS bridge.** Package
   `com.apple.android.music.commerce` owns `WebViewModel` +
   `AccountCreationWebViewModel` + `AccountCreationActivity`; the JS interface
   `commerce/jsinterface/account/PrimaryAccount` exposes `getAppleId()`,
   `getDsId()`, `getUserName()` to the page. So the flow is: bag-provided URL →
   Apple's own web auth (idmsa behind it) → page calls back into native for the
   account identity. Reusing this flow means running a real webview inside the
   auth origin — a native REST replay is not what the app does.

3. **`music-user-token` is minted by StoreServices, not the public API.**
   `sdk/SDKAuthViewModel.getMusicUserToken(developerToken)` awaits
   `storeservices.data.UserTokenResponse`, produced in the `storeservices`
   package whose behavior lives in `libstoreservicescore.so` (JNI bridge:
   `storeservices/javanative/...`). That native layer carries `passwordToken`,
   `X-Apple-Store-Front`, and `primingSignatureForData` — i.e. the private
   iTunes/StoreKit account protocol with Mescal request signing
   (`shouldMescalPrimeRequestsForURL` in the same lib) and device provisioning
   (`libCoreADI.so`). A third-party client cannot mint this token from
   `amp-api.music.apple.com` — the public MusicKit user-token flow
   (`developer token + user auth`) exists but is a *different, weaker* path
   (see §what's still legally reachable).

4. **FairPlay is client-side native and complete.** `libCoreFP.so` (8.2 MB) is
   the FairPlay implementation; `jsinterface/helper/FairPlayContext.java`
   exposes provisioning context to the commerce webview. Protected HLS uses
   `hls-key-server-url` / `hls-key-cert-url` / `hls-playlist-url` — all bag keys.
   So the decryption boundary is: Apple-signed key server + FairPlay client +
   ADI-provisioned device identity, all inside the app.

## What this closes and what it leaves open

| question | verdict | evidence |
| --- | --- | --- |
| Can a native client reuse the app's auth path? | NO without reimplementing StoreServices+Mescal+ADI (private protocol RE) | decompiled chain above |
| Can catalog/metadata be touched legally? | YES — MusicKit REST with a developer token (account-gated: paid dev membership) | unchanged from route doc |
| Is hi-res streaming reachable natively? | NO — FairPlay client+key server+ADI are all inside the app; implementing one = DRM circumvention (out of scope permanently) | `libCoreFP`, hls-key bag keys |
| Could the same creds drive a Linux webview for AAC? | technically the web path exists (Widevine) but is out of scope: embedded browser = the weight this project removes | spec non-goals |

## Notes for whoever picks this up next

- `evidence/native-scan/hosts.txt` lists the only hardcoded hosts: bag +
  cert-authority pages + cookie domains. Everything else is bag-delivered.
- `libmediaplatform.so` / `libmedialibrarycore.so` hold playback/library
  internals; further RE would only refine HOW closed it is.
- Windows Apple Music client is built from the same media core (public
  reporting, unverified here); same StoreServices/FairPlay conclusion expected.
  MSIX fetch attempt via rg-adguard returned nothing 2026-10-15 — retry with a
  different acquisition route if the Windows binary is still wanted.
- `tools/axml.py` `package: ?` **fixed** — parser wrapped output in a synthetic
  root so real `<manifest>` attrs were never read; `package:
  com.apple.android.music` now verified.
- Design-donor licences verified via GitHub API (2026-10-15): both
  `lethenju/spotify_rust` and `SinLeeke/velo` are **MIT** — they upgrade from
  REFERENCE_ONLY to code-reusable donors. `Sidra` is confirmed **Electron**
  (extracted AppImage: chromium+app.asar, loads `music.apple.com`) — i.e. it is
  the webview approach this project exists to avoid, not a native reference.
- `skillscript.ci` referent: closest public match is `sshwarts/skillscript`
  (TypeScript declarative workflow language, active 2026-10-12) — confirm with
  user whether that is the intended CI tooling before wiring slice 06.
