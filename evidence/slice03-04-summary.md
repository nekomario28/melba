# Slice 03 (MPRIS + CLI) / 04 (native UI) — verification summary 2026-10-15

## Slice 03 — org.mpris.MediaPlayer2.aml

Runtime: session bus, daemon `aml serve <queue> --device auto` owns name
`org.mpris.MediaPlayer2.aml`; `aml ctl <verb>` + any MPRIS client drive it.

- `ctl status/play/pause/next` all functional — transcript:
  `evidence/slice03-mpris-transcript.txt` (Playing→Paused→Playing, Next advances queue)
- `busctl get-property … PlaybackStatus` → `s "Playing"`; `Position` → `x <µs>` (int64 per spec)
- Exported: PlaybackStatus, Position, Metadata (xesam:title + mpris:trackid), Can*, Volume, CanQuit, Identity, SupportedUriSchemes/MimeTypes
- Real audio path is the slice-01 engine — Play actually opens hw: device at source rate.

### Findings during bring-up
- **Position was exported as u64** — MPRIS spec says `x` (int64); fixed, verified `x` on wire and in UI.
- **Second `aml serve` does not take the name** — first owner keeps
  `org.mpris.MediaPlayer2.aml`; second instance plays silently without owning the
  well-known name. Acceptable for now; documented, not silently claimed.

## Slice 04 — aml-ui (eframe/egui, MPRIS client only)

- Window renders under both Wayland (live session) and X11/Xvfb (gate path).
- Polls PlaybackStatus/Metadata/Position every 250 ms over dbus — **UI knows the
  daemon only through the public MPRIS contract** (separation proof).
- Screenshot (Xvfb + import): `evidence/slice04-ui-playing.png` — title, state,
  live position, transport buttons visible.
- Memory: UI PSS ≈ 114 MB / RSS 128 MB (eframe+wgpu on X11); daemon `aml serve`
  PSS ≈ 14 MB. "Lightweight" claim holds for the audio path; the UI carries
  wgpu weight — acceptable, flagged honestly.

## Boundary unchanged
No Apple account, no FairPlay path, no protected-catalog streaming. Library =
local files via `sone.collection/v1`.
