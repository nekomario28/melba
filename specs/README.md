# specs/apple-music-linux — a native, lightweight hi-res music client for Linux

## Active desktop work

[Desktop lifecycle and update UX](desktop-lifecycle-update/plan.md) tracks the
current close/Quit, startup, MPRIS forwarding and update-adapter acceptance.
The dated slice history below is not its build/install receipt.

## Next Agent Prompt

Status as of 2026-10-15 (post-slice): **all six slices landed and gated.**
The client plays owned FLAC/ALAC bit-perfect to an exclusive `hw:` sink
(ffmpeg-referenced decode equality + BITPERFECT receipt), serves full MPRIS
transport with signals/art/unique trackids, ships an egui UI (wgpu renderer;
Wayland-verified on this host's KWin/amdgpu) with Now Playing / Library /
Search / Queue / Album views, MusicKit catalog search + preview streaming,
and `en`/`ja` system-locale i18n. Post-slice additions: software Volume
(≥0.999 passthrough keeps the bit-perfect path exact), keyboard transport,
gapless stream reuse across same-format tracks (`melba:streamOpens` counter
proves opens stay < track count), duplicate-daemon guard on the bus name,
CJK font fallback via fontconfig, `--demo-shot` in-app screenshots, and a
docgx layout gate (`gx.aml_ui_layout (removed)`). Queue persistence
(`~/.local/state/melba/queue.json`, atomic write, restored by bare `melbad
serve`), playlist/artist/settings views, queue context menu, runtime
output-device switching (`melbad ctl device hw:X,Y|auto` / `MelbaSetDevice` —
current track restarts on the new sink; `melba:device` + `melbad now` expose the
request), and `.desktop`/icon/user-unit packaging are all landed. The
device picker always offers `auto`, plus `default` — the ALSA plug into
PipeWire/PulseAudio: the stream follows OS-level output switching and shares
the sink with other apps; verdict honestly reports `SHARED` there since an
OS resampler can sit past the hw: boundary. Note: a resident `melbad`
holds the MPRIS bus name, so `ci/check.sh` MPRIS gates must run with the
user unit stopped (dup-guard working as designed). Also
fixed: `ctl stop` used to share Next's code path and skipped a track instead
of halting (`mpris.stop-halts` gate). Compact mini-player mode is landed
(superseded: the `melbad-ui` binary, its compact mini-player, deep-link
handling, and `ui-links.sock` were removed with it; the Melba app in
`crates/melba/` is now the UI. catalog pages carry an
"Open in Apple Music" reverse handoff for licensed full-length playback.) Lyrics UI is
landed for catalog previews: Now Playing shows a lyrics button when the
track uri is `preview:<id>`; `View::Lyrics` fetches TTML off the paint path,
converts via `musickit::ttml_to_text`, and renders scrollable text. The
endpoint is user-token gated: without `melbad auth` Apple answers 400 and the
UI shows that verbatim — live lyric content is **NOT_RUN** in this
environment. The public `evidence/ui-lyrics-*.png` files are now
[synthetic illustrations](../evidence/README.md), not proof of TTML rendering,
live lyrics or playback.
Windows port: **declined** — the repo exists
because Linux has no native client; Windows has Apple's own, and no Windows
test host exists here (an unverifiable port would violate evidence-first).
The architecture stays portable (audio:: sink boundary, wgpu→DX12, MusicKit
is plain HTTP) so the decision is cheap to revisit. The device-switch
feature above was the lesson taken from the Windows app's reviews: playback
stalls and launch-time-only output selection were its top complaints.
Global TODO: [x] 01–06 all PASS on hardware
(`ci/check.sh --hardware`, 17 gates). Update this section before ending your
pass.

## Goal

A **native** Linux music client that is genuinely lightweight and genuinely
hi-res: it plays audio the user already owns at the source's own sample rate
and bit depth, through an output path with no resampler in it, and it behaves
like a native player (MPRIS, playerctl, media keys, CLI). It is the lightweight
Apple-Music-*shaped* client the user asked for — same mental model (library,
playlists, now-playing, quality tiers), honest about which Apple surface it can
touch.

## The boundary, stated once, enforced everywhere

- **Apple Music streaming hi-res is not reachable on Linux without Waydroid.**
  Hi-res lossless delivery is FairPlay-protected; Apple licenses FairPlay
  clients only to its own apps, and Linux browsers do not ship FairPlay EME, so
  web/Servo/WebView embeds top out at AAC through Widevine (measured in
  `~/work/rust-browser-lab`: no `MediaSource`, `requestMediaKeySystemAccess`
  undefined). Implementing a FairPlay client would be DRM circumvention —
  **out of scope permanently, not deferred.**
- Therefore this client never claims to stream protected catalog audio. It
  plays what it can prove: local files (FLAC/ALAC/WAV/DSD via ffmpeg or
  symphonia), DRM-free purchases, and anything a legal importer produces.
- The Waydroid container remains available to the *user* for protected
  playback, and this client must not pretend to replace that. It may consume
  ordinary PipeWire streams like any other player (e.g. control the host graph
  rate), but it does not decrypt anything.

## What is already true (measured, or from prior portfolio work)

- The *host rate policy* problem is solved and evidenced in `~/Documents/HiRes`:
  `clock.force-rate` following, stream locking, device-capability clamping
  (HDMI ELD / HD-Audio codec rates / `aplay --dump-hw-params` probe), and the
  measured decomposition showing the resample-through-48k graph is what silently
  destroys hi-res. Slice 01 must reuse that knowledge, not rediscover it.
- The same repo states the target explicitly: *\"for strict bit-perfect playback,
  a native Linux player with an ALSA/PipeWire path designed for exclusive or
  fixed-format playback is still the better target.\"* That sentence is now this
  project's reason to exist.
- `~/work/rust-browser-lab` supplies measurement discipline (cgroup-scoped PSS,
  evidence JSON, no claims without a receipt) — and the negative result that a
  Servo/web embed cannot carry protected or adaptive audio. Borrow the
  discipline; do not borrow the engine.
- Spotify Rust rewrites (`lethenju/spotify_rust`, `SinLeeke/velo`) are design
  donors only — licences unverified, so `REFERENCE_ONLY`: shape and rationale,
  never code.
- Importers and the frozen `sone.collection/v1` contract already exist in
  `/data/sone`; slice 02 extends the importer set (XSPF, iTunes/Music XML)
  behind that contract rather than inventing a second one.

## Non-goals (hard)

- No Waydroid, no Android runtime, no APK execution, no container of any kind.
- No DRM circumvention, FairPlay/Widevine client work, or DRM-key handling.
  Explicitly authorized MusicKit user tokens may be read from the user's sign-in
  and stored locally with private permissions; unrelated browser profiles and
  cookies are not inspected.
- Apple ID password/MFA collection is excluded except for the maintainer-approved
  local managed-wrapper GUI sign-in scope in [Unified sign-in](unified-signin.md).
  That flow must preserve Apple's offered device/phone verification choices and
  keep credentials transient. This is approved scope, not shipped behaviour.
- No reimplementation of an existing authority: not a second PipeWire, not a
  second scrobbler, not a second library model when `sone.collection/v1` fits.
- No second language for aesthetics — Rust because the host already has a Rust
  audio/UI toolchain and the Servo lab's measurement harness; a native UI crate
  (egui/iced-class) rather than an embedded browser, because an embedded browser
  is exactly the weight this product exists to remove.

## Slice graph

Order is risk-first: 01 can falsify the whole claim, 02 makes it useful, 03
makes it native-feeling, 04 is where taste lives, 05 is enrichment behind a
paid-account gate, 06 keeps every gate honest per commit.

| # | slice | contract it unlocks | verdict artifact |
| --- | --- | --- | --- |
| 01 | bit-perfect core ✅ | source format reaches the device unconverted | `hw_params`/PipeWire format receipt + a *negative* case proving a 48k graph does not silently pass |
| 02 | library + importers ✅ | own files become a queryable library behind `sone.collection/v1` | fixture-driven importer tests, no Apple account needed |
| 03 | MPRIS + CLI transport ✅ | desktop keys / `playerctl` / scripts control playback | `busctl --user` transcript — done against the real player, `evidence/slice03-mpris-transcript.txt` |
| 04 | lightweight native UI ✅ | now-playing, queue, quality readout | runnable window + PSS receipt — `evidence/slice04-ui-playing.png`, UI PSS ≈114 MB, daemon PSS ≈14 MB |
| 05 | MusicKit metadata ✅ | catalog search, art, hi-res availability flags | public dev token from music.apple.com bundle (AMPWebPlay), catalog calls verified — `evidence/slice05-musickit.md`; `/v1/me/*` implemented separately, account-dependent verification NOT_RUN |
| 06 | CI gates ✅ (local) | slices 01–05 gates run per commit | `ci/check.sh [--hardware]` — 17 gates, all PASS on this host; skillscript.ci referent still unfound |

## Feature surface vs official Apple Music

| feature | status | how |
| --- | --- | --- |
| Local hi-res playback | ✅ | symphonia → ALSA `hw:` exclusive, BITPERFECT receipts |
| Gapless playback | ✅ | ALSA stream survives same-format track changes (no drain/reopen between tracks); format changes drain-then-reopen; `melba:streamOpens` exposes the real open count |
| Volume | ✅ | MPRIS `Volume` get/set + `ctl volume` + UI slider; gain ≥0.999 is an exact passthrough so the unity path stays bit-perfect |
| Locale/i18n | ✅ | `melba` follows the OS locale or the Settings picker; 16 gettext catalogs ship in-tree, CJK glyph fallback via fontconfig. The deleted `aml-ui` had its own `AML_LANG`/`en`+`ja` surface — superseded |
| Daemon lifecycle | ✅ | duplicate `melbad serve` refuses the owned bus name; `Quit`/`Raise` implemented |
| Queue persistence | ✅ | `~/.local/state/melba/queue.json` written atomically on enqueue/advance/quit; bare `melbad serve` resumes the saved queue+position index |
| Decode bit-exactness | ✅ proven | `tests/bitexact.rs`: symphonia→i32 output byte-identical to `ffmpeg` s32le/s16le for ALAC 16/24-bit + FLAC 16/24-bit (`fixtures/88k-16bit.m4a`, `96k-24bit.m4a`, …); S16LE downshift exact. Evidence: `evidence/alac-bitperfect-media-controls.md` |
| Transport | ✅ | Play/Pause/PlayPause/Stop/Next/Prev + **Seek/SetPosition/LoopStatus/Shuffle/OpenUri/Quit/Raise** over MPRIS; `Seeked`+`PropertiesChanged` signals emitted; unique `mpris:trackid`; `xesam:title/artist/album/url` + `mpris:artUrl` (embedded-cover extraction to `~/.cache/melba/art/`, catalog art for preview/collection, cleared when absent) |
| Catalog search/lookup | ✅ | `mk search|song|album`, `extend=audioVariants` (lossless/hi-res-lossless flags) |
| Catalog preview streaming | ✅ | `melbad preview <id>` — public 30s AAC → real playback path (measured BITPERFECT) |
| Downloads | ⚠️ partial | `melbad dl <id>` = preview + artwork + manifest to `~/.cache/melba/downloads/`; **protected full-track offline is FairPlay-bound — not implemented, by design** |
| User library read (`me.*`) | ✅ implemented, ⏸ NOT_RUN | `mk me library-*|recent-*|recommendations|heavy-rotation` — needs `melbad auth login` or a manually supplied token |
| Library write (add) | ✅ implemented, ⏸ NOT_RUN | `mk add songs|albums <id>` — official POST /v1/me/library |
| Auth | ✅ implemented | `melbad auth open` = in-app WebKitGTK sign-in → `media-user-token` cookie captured from the webview jar (works despite MusicKit reporting `restricted` — cookie is issued by the redirect chain; verified `/v1/me/storefront` → 200). `--browser` = normal-browser localStorage watch. Fallbacks: `auth set`, or the `melbad auth how` bookmarklet (shows the token to paste — the old `aml://auth` deeplink receiver was removed with `aml-ui`). |
| Subscription playback (`stream:`) | ✅ verified | `stream:<song-id>` → AAC-256: `melba-cdm` hosts your installed Widevine CDM, licenses via Apple's `webPlayback` endpoints, decrypts `ctrp` fMP4 in-process — verified end-to-end live. ALAC: `enhancedHls` `CODECS=alac` + `cbcs` decryption through a user-supplied wrapper (`wrapper_url`, or `wrapper_mode:"managed"` — x86_64 native verified) |
| Protected offline | ❌ by design | FairPlay keys never leave their licensed context; `melbad dl` ships preview+manifest only |
| Sidra hand-off | ✅ verified | `tools/sidra-bridge.py` offloads playback from Sidra's MPRIS trackids into melbad (`stream:`/`preview:`), mutes Sidra, mirrors transport — see README |
| Radio/lyrics | ⚠️ partial | `mk lyrics` implemented — needs `melbad auth` user token (dev token gets 40012); stations searchable via `mk search --types stations` |
| Web-player "Open in Music" | ✅ restored | `melba.desktop` registers `x-scheme-handler/{itms,itmss,music,musics,apple-music}` with `Exec=melba %u`; `link::parse` accepts those schemes plus `music.apple.com` share links (Apple host only, `?i=` song links) and a running instance receives the forwarded link over its single-instance socket |

## Agent-native surface

Every state/IO path an agent needs is JSON or a plain pipe — no TTY scraping:

- `melbad about` — capability manifest (commands, machine interfaces, stated boundaries). Self-discovery without reading docs.
- `melbad now` — full player state as one JSON object (status, positionMs, durationMs, title, artist, album, artUrl, uri, queue[], queueIndex, shuffle, loopStatus, sink, verdict). Sources MPRIS Metadata directly.
- `melbad mk search "<q>" --collection` — catalog search emitted as a `sone.collection/v1` doc whose `playUri`s are `preview:<apple-id>`.
- `melbad mk collection <albums|playlists> <id>` — any catalog album/playlist → serve-ready collection doc (tracks carry `apple.audioVariants`, `apple.artwork`, `apple.catalogId` props).
- `melbad serve <collection.json|/dev/stdin>` — content-sniffs each arg; a collection expands to `playUri`s and seeds the title map (`Artist — Title` flows into MPRIS `xesam:title`). `preview:<id>` entries resolve inside the playback thread to the cached 30s AAC download (`musickit::download_preview`), then run the same ALSA `hw:` path — measured `BITPERFECT`, `dur ~30023ms`.

The discovery loop that falls out of this, no glue code required:

```
melbad mk search "toto africa" --types songs --limit 8 --collection > mix.json
melbad serve mix.json          # or: ... --collection | melbad serve /dev/stdin
melbad now                     # {"title":"Toto — Africa","status":"Playing","verdict":"BITPERFECT",...}
melbad ctl next / seek 15 / shuffle on
```

Verified end-to-end on hw:0,0 (search→collection→serve→now→ctl pause/play, 2026-10-16).

## Standing contracts

- **Resource discipline** (host-global rules): cargo builds and any DSP-heavy
  probe are not \"light\" — heavy lease (`resource_lease.py acquire --class
  heavy`), backgrounded `systemd-run --slice=devin-compute -p MemoryMax=`,
  immediate bind. Unit tests, importer fixtures and the MPRIS transcript are
  light: `-p MemoryMax=<need>` plus a fresh `resource_gate.py --class light` GO
  in the same command.
- **Evidence discipline**: a claim about sound exists only as a receipt in
  `evidence/*.json` produced by the harness — device format, delivered rate,
  process-group PSS. \"Hi-res\" and \"lightweight\" are numbers or they are not
  claimed.
- **Bit-perfect honesty** (carried from HiRes): monitor samples prove audio
  reached the host sink; they do not prove the receiver, amp or speakers
  reproduced it. Every receipt states that boundary.
- **Provenance**: no binaries, no third-party patches, no Apple artifacts in
  this repo. Scripts, docs, fixtures, checksums only.
- **Commits**: explicit pathspecs only (shared-index guard), no AI attribution
  trailers.

## Known gaps in this research pass

- `tools/axml.py` `package: ?` fixed 03:1x by a concurrent session — the parser
  wrapped output in a synthetic root so the real `<manifest>` attrs were never
  read; now reads the real manifest node (`package: com.apple.android.music`
  verified against the installed APK).
- Native `lib/` inside the APK was never scanned by this pass (no longer on the
  critical path after the pivot). A concurrent session left
  `evidence/native-scan/*.endpoints.txt` and `evidence/slice01-flowing.json` in
  `evidence/` at 03:07–03:09 — the first is useful but `UNVERIFIED` (see the
  research doc), the second is a **container-path** receipt and does not satisfy
  slice 01. Neither is deleted; another session's evidence is not mine to remove.
- Whether the app's declared `MediaBrowserService` answers companion apps
  without a signed-in session: untested device question, and now belongs to a
  future handset companion, not to this repo.
- DSD-over-PCM (DoP) and bit-depth >24 handling on this host's HDMI path:
  `UNKNOWN`, and it is a slice-01 sub-question, not an assumption.
