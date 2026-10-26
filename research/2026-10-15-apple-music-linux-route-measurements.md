# Apple Music on Linux — measured route surface (2026-10-15)

Static, read-only evidence for "can a lightweight Linux client play Apple Music at
hi-res?" No target execution, no network calls, no account use. Every row below is
either `MEASURED` (something was read) or `NOT RUN` (design only). Unknowns stay
`UNKNOWN`.

## Artifact identity

| item | value |
| --- | --- |
| APK | `~/.local/share/waydroid/data/app/~~MIPWwfMe_ZfGAeoD59Dp-g==/com.apple.android.music-kIMKillxusC8-fZwUlv6tA==/base.apk` |
| sha256 | `ecbd223130e3bc4429402239f77c0ef6a1bcd2860a124630d6fa04eb72defa08` |
| size | 142,669,295 B (dir 222 MB with `lib/` + `oat/`) |
| read via | `tools/axml.py` (pure-Python AXML) + `unzip -p classes*.dex \| grep -a` |
| NOT used | REA `inspect_android_package` — needs `REA_JADX_MCP_JAR` (jadx-headless-mcp 0.7.1), not installed and REA never installs engines |

Why the hand-written parser: this host has no `aapt2`/build-tools, so
`apkanalyzer manifest print` fails with "Cannot locate latest build tools".
`tools/axml.py` answers the manifest questions in ~120 lines of read-only Python
and prints nothing it did not read.

## What the manifest says

- **`com.apple.android.music.player.MediaPlaybackService` is `exported=true`** —
  the only Apple-owned exported service. This is the session the existing
  `~/Documents/HiRes/waydroid-mpris-bridge.py` already drives through
  `adb shell cmd media_session dispatch <verb>`. It is the transport seam a native
  front-end can use today, with no new reverse engineering.
- **21 services total, and exactly one declares `android.media.browse.MediaBrowserService`:**
  `com.apple.android.music.player.MediaPlaybackService`, `exported=true`, **no
  permission**. Its intent filters are `MediaBrowserService` +
  `intent.action.MEDIA_BUTTON`; `androidx.media.session.MediaButtonReceiver` is
  also exported. This *reverses* the `UNKNOWN` recorded in `§1d` of
  `/data/sone/docs/APPLE-MUSIC-ROUTES.md` on the static half of the question:
  the app **declares** a browsable service, so an account-free library browse is
  not structurally excluded. What the declaration does **not** prove is that it
  answers browse requests from a companion package without an authenticated
  session — that stays a device gate (`dumpsys media.browse`, or a real
  `MediaBrowserCompat.connect` on the handset) and is `NOT RUN` here. Do not
  upgrade this to "library reads work" without that runtime receipt.
- Exported activities include `UriHandlerActivity` and `SplashActivity` — i.e.
  `music://` / `itunes` URI handling, which is what `MPRIS.OpenUri` already relies
  on. No exported surface implies an open write path.
- 22 permissions, all ordinary for a streaming app (`INTERNET`, `FOREGROUND_SERVICE`,
  `MODIFY_AUDIO_SETTINGS`, `READ_MEDIA_IMAGES`, `CAMERA`, GMS/Firebase, Play
  BILLING). Nothing that widens the trust boundary for a companion app.

## What the bytecode says about playback and auth

Token counts over `classes*.dex` (ASCII, case-sensitive, `grep -a -o -c`):

| needle | hits | reading |
| --- | --- | --- |
| `FairPlay` | 12 | DRM path present in the app itself → lossless/hi-res is FairPlay-protected, not plain progressive download |
| `fairplay` | 2 | lower-case variant (framework/loader names) |
| `api.music.apple.com` | 3 | catalog/API host used by the app |
| `amp-api` | 3 | Apple Music web API namespace — the same API the web client uses |
| `atmos`, `lossless`, `spatial`, `hi-res` | 5 / 3 / 3 / 1 | quality tiers exist client-side (matches `AudioQualitySettingsActivity` in the manifest) |
| `musickit` / `MusicKit` | 0 | no MusicKit SDK string in dex (native libs not scanned; absence here is not proof of absence) |
| `idmsa` | 0 | Apple ID sign-in host string not in dex — also in native libs or built dynamically |
| `store.itunes` | 0 | — |
| `playAuthenticationToken` | 0 | — |

Consequence for the design: the app's protected playback depends on FairPlay and on
an authenticated Apple session held **inside the Android container**. Any Linux-side
reimplementation of that decryption would be DRM circumvention and is out of scope
for this project, permanently and by policy — not "later".

## Route matrix (reuse-first, from existing portfolio work)

| route | what it gives | status |
| --- | --- | --- |
| Waydroid Android app + `~/Documents/HiRes` patch (direct outputs) + rate follower | real 96/192 kHz PCM to host PipeWire | MEASURED working on this host (HiRes README: lineage-20, Apple Music 4.8.0, HDMI→RX-V6A) |
| MPRIS bridge over `MediaPlaybackService` | transport control from Linux media keys / any MPRIS client | MEASURED present (`OpenUri`, `Play/Pause/Next`, `setVolume`) |
| Native thin front-end (Rust) instead of the Waydroid window | the "lightweight client" the goal asks for; Waydroid keeps only audio, no UI session | NOT RUN — this is slice 1 |
| MusicKit / Apple Music API | library + playlist reads, developer-token streaming URLs | NOT RUN, blocked on paid Apple Developer membership (per `/data/sone/docs/APPLE-MUSIC-ROUTES.md` §1/§1b) |
| Web client / Servo/WebView embed | nothing at hi-res: Widevine→AAC only | MEASURED blocker in `~/work/rust-browser-lab` (`docs/mse-decision.md`: no `MediaSource`, `requestMediaKeySystemAccess` undefined) |
| Spotify Rust rewrites (`lethenju/spotify_rust`, `SinLeeke/velo`) | design donors for a small Rust music UI | REFERENCE_ONLY — licences not verified; design and "why" only, no code reuse |
| scrobble/history (ListenBrainz/Last.fm) + local importer | preference data with no vendor account | REFERENCE_ONLY donor idea; does not read declared playlists |

## Consequence of the 2026-10-15 pivot: no Waydroid (user decision)

The user chose a **native client with no container**. Everything above is still
true about the APK, but it stops being a usable playback path, because the
FairPlay-protected pipeline lives inside the Android runtime. What survives:

- **The transport/browse findings above become the reference for a *device*
  client, not for this one.** Keep them; do not delete them — they are the
  measured answer to "what could the container do", and they are what a future
  handset companion would use.
- **Apple catalog hi-res becomes unreachable on Linux by construction.** Hi-res
  lossless delivery is FairPlay-protected; the only FairPlay clients Apple
  licenses are its own apps (and FairPlay EME on platforms that ship it, which
  Linux Chrome/Firefox do not). Reimplementing that client is DRM circumvention
  and is out of scope permanently — not "later".
- **What "hi-res on Linux" can honestly mean here** is the sentence already in
  `~/Documents/HiRes/README.md`: *a native player with an ALSA/PipeWire path
  designed for fixed-format playback*. That target is now this project's slice
  01, and it applies to audio the user already owns (local files, DRM-free
  purchases). The HiRes scripts solved the *host rate policy* half of it
  (`clock.force-rate`, stream locking, device clamping); this project owns the
  *player* half so the rate is chosen by the source and never resampled.
- **The Apple Music side degrades gracefully and stays useful**: catalog
  metadata/search through MusicKit (needs a paid Apple Developer membership —
  `NOT RUN`, gated) for enrichment and cover art, plus playlist *import* through
  the frozen `sone.collection/v1` contract. No playback of protected catalog
  audio is claimed anywhere in this repo.

Security boundary unchanged and now simpler: there is no container to protect,
so the boundary is "no credentials in this repo, no token exfiltration, no
FairPlay/DRM implementation, no scraping of an authenticated session".

## Open questions this research does not answer

- Whether `MediaPlaybackService` accepts `playFromSearch` from a companion package —
  that is a device gate on the handset, not a manifest question. `UNKNOWN`.
- Native-lib contents (`lib/`) — `MusicKit`/`idmsa` may live there. **Partially
  filled by a concurrent session, unverified:** `evidence/native-scan/*.endpoints.txt`
  appeared at 03:07–03:09 (after this pass was written; I did not run it, cannot
  vouch for its method — `UNVERIFIED` until reproduced). Cross-checked only for
  existence against the APK: `libCoreFP.so` (8.2 MB) is the **FairPlay client**
  (CA/CRL strings) — which makes the no-Waydroid boundary structural, not a
  tooling gap; `libstoreservicescore.so` (2.4 MB) is Apple's StoreKit framework
  cross-compiled for Android (NDK build path in strings);
  `libmedialibrarycore.so` (13.0 MB) supplies nearly all endpoint strings. Domain
  inventory: `www.apple.com` ×32, `init.itunes.apple.com`, `sandbox.itunes.apple.com`,
  and leading-dot forms `.music.apple.com` / `.itunes.apple.com` whose shape reads
  as a suffix/pinning table — do not quote those as endpoints. `idmsa` still does
  not appear, so the dex-side `idmsa = 0` reading remains inconclusive.
- Whether a user-token MusicKit request clears library endpoints from an
  Android-supplied token. `UNKNOWN` — needs one authenticated request, needs the
  paid account first.
