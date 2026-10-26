# Spotifast UI adoption (a-lite) — plan

Goal: replace aml-ui's egui frontend with spotifast's (crmne/spotifast, MIT,
egui/eframe 0.36 + fastframe-* crates). aml daemon stays the audio engine;
the new UI is a separate crate that talks to it over MPRIS/ctl.

## Why a-lite (keep types, swap semantics)

- `model.rs` (1.4k) + `api/models.rs` (1.1k) are the UI contract; `Track.uri`
  is a plain String — carries `preview:<cc>:<id>` / `file:///` unchanged.
- `Track.is_local` exists; local library maps natively.
- ui/ emits `Action`s applied in app.rs — semantics swap at the handler seam.
- aml extras (device picker/bit-perfect, auth sync, storefront, deeplink log)
  land as one added `Page` variant — the designed extension point.

## Layer disposition (measured 2026-10-15, main@HEAD shallow clone)

| layer            | LOC   | disposition                                    |
|------------------|-------|------------------------------------------------|
| src/ui/ + skin/  | ~60k  | keep as-is                                     |
| src/model.rs     | 1.4k  | keep mostly (Spotify-flavored strings reused)  |
| src/app.rs       | 27.4k | adapt (~60% reuse); cut Connect/session/podcast|
| src/api/         | 3.7k  | rewrite -> MusicKit catalog                    |
| src/backend.rs   | 8.7k  | rewrite -> aml-daemon proxy (MPRIS + ctl IPC)  |
| src/player.rs    | 1.7k  | rewrite -> MPRIS transport proxy               |
| deps             | —     | drop librespot x5, projectm, rodio?, winamp    |
|                  |       | keep fastframe-*, eframe 0.36, reqwest         |

## Phases

- P0 scaffold: `crates/aml-ui2` (or `src/amlui/`) copy of spotifast tree,
  compile against stubbed backend/api (biggest single step — type surface).
- P1 backend: MPRIS transport/metadata/queue proxy to aml daemon.
- P2 api: MusicKit catalog (search/album/artist/playlist/collection).
- P3 library: aml local library scan -> their Library/Page views.
- P4 aml tab: device picker + bit-perfect verdict, `aml auth sync` status,
  storefront picker, deeplink log.
- P5 integration: deep-link `%U` + socket forward, single-instance
  (fastframe-instance may already provide it), MPRIS identity `aml-ui`,
  ja/en i18n (fastframe-i18n), packaging, evidence screenshots.
- P6 decision: replace aml-ui default or keep both binaries.

## Risks / honest boundaries

- app.rs is 13x the current aml-ui; multi-session effort.
- fastframe-* git deps pinned by rev; Wayland frame-pacing fix may also
  resolve our glow-freeze history.
- MIT: keep copyright + LICENSE text; repo keeps its own CI (strip theirs,
  plus their AGENTS/CONTRIBUTING rules bind contributors to upstream, not us).
- Spotify-only features (podcasts/shows, prerelease, Connect remote,
  milkdrop) are cut first; milkdrop can return later if we expose PCM frames.
- Public-preview-only reality unchanged: subscription full playback is still
  not possible; the "Open in Apple Music" reverse handoff stays.
