# Wayland verification + UI visual review — 2026-10-15

This is a historical observation record, not a fresh acceptance run. The album
and CJK PNGs named here are now [synthetic documentation illustrations](README.md).
They are not the original captures and do not establish renderer, service or
audio behaviour.

## Backend result (this host: KWin wayland, amdgpu card0+card1, eframe 0.31.1)

| backend | session | result |
|---|---|---|
| glow (GL) | wayland / XWayland / Xvfb | **dead** — freezes after 2 frames (host mesa/amdgpu issue, reproduces in a 20-line hello-egui) |
| wgpu (Vulkan) | wayland-0 (real session) | **works** — frames advance continuously (80+), `ViewportCommand::Screenshot` returns real pixels |
| wgpu | X11 via XWayland | works — same continuous frames |

AML_RENDERER=glow|wgpu selects the backend; default = wgpu when the feature
is enabled. Evidence of "works": not inference — `AML_SHOT_TO` captured a real
rendered frame of the running app under Wayland (the original album/CJK captures
are not in the current public tree), and
the frame counter ran to 80+ where the glow path stalled at 2.

Boundary: proven on this compositor/GPU combo. "Wayland完全OK" in the sense
that there is now a verified working path on this host — not a claim about
every compositor/driver matrix.

## AML_SHOT_TO (new verification surface)

`AML_SHOT_TO=<png> [AML_SHOT_DELAY_MS] [AML_VIEW]` — the app requests
`egui::ViewportCommand::Screenshot`, receives `Event::Screenshot` (implemented
by both glow and egui-wgpu's `handle_screenshots`), saves PNG, closes.
Compositor-independent: no portal, no window focus, no Xvfb needed.
eframe's own `EFRAME_SCREENSHOT_TO` is glow-only in 0.31 — do not use.

Captured this session (wgpu, real Wayland, daemon on hw:0,0):
- Now Playing: embedded art rendered at 220px, tag title, Paused, BITPERFECT
- Queue: 3 entries, current highlighted "▶ aml fixture — Bit Test"
- Library: 10 tracks with rate/bits badges from scan JSON

## Bugs the screenshots exposed (all fixed)

1. `want_art` fetched art only via HTTP (ureq) — `file://` URLs (embedded
   covers under ~/.cache/aml/art) never rendered. Now reads local files.
2. `aml-ui lib.json` / any `.json` import → "unknown playlist format":
   `library::import` dispatched on extension only and had no sone.collection
   branch, so the documented `aml scan > lib.json` → `aml-ui lib.json` flow
   was broken. Added `.json` → sone.collection/v1 parse (Deserialize derives).
3. Library rows rendered "— <title>" for artist-less tracks; nav 110px
   clipped "Now Playing" → widened to 130, artist prefix now conditional.

## docgx layout check (mathematical verification)

`ci/models/aml_ui_layout.py` declares the Now Playing geometry (nav 130px,
transport ~92px, center content) as docgx blocks; `docgx inspect` reports
**0 errors** — no clipped text, no overlaps at declared geometry.
Storyboard: `evidence/aml_ui_layout.storyboard.png`.
Boundary: docgx checks declared boxes, not egui's runtime text metrics —
it is a spec-level sanity check. Runtime text metrics require an actual capture;
a synthetic illustration does not supply that evidence.
manimgx is an animation renderer; docgx is the gxkit tool that actually
verifies fixed-size layout math.

## Gates

`cargo test --release` 14/14 · `ci/check.sh --hardware` 14 PASS 0 FAIL
(incl. hires.bitperfect on hw:0,0).

## Parity slice B (this session's follow-up)

- **Volume** — was a stub (always 1.0, set ignored). Now: atomic gain in the
  playback path (`audio::apply_gain`; >=0.999 = exact passthrough so the
  BITPERFECT claim is untouched), MPRIS `Volume` get/set with
  `emits_changed_true` (observed PropertiesChanged DOUBLE 0.7 on the bus),
  `aml:volume` metadata, `aml ctl volume <0-100>`, `aml now` field, UI slider.
- **Keyboard transport** — Space = PlayPause, ←/→ = Seek ±5s, ↑/↓ = Volume
  ±0.05; suppressed while a text field owns focus.
- **Album view** — song -> `?include=albums` -> `collection_of` -> numbered
  tracklist with artwork + audioVariants badges; each row is preview-playable.
  The similarly named public `ui-album-wayland-wgpu.png` is a synthetic
  illustration, not evidence for this historical live observation.
- **Bridge status** — sidebar shows `bridge: on|off` by probing
  `org.mpris.MediaPlayer2.applemusic-web` via NameHasOwner in the poll worker.

Fixed while here: `sone.collection` docs from `collection_of` lacked
`provenance.retrievedAt` -> strict Deserialize rejected them; field is now
`#[serde(default)]`.

CJK gap FIXED (same session): `install_cjk_fonts` resolves a system font via
`fc-list file:face-index` (exact TTC face) — Noto Sans CJK JP preferred,
`:lang=ja` fallback — appended to egui's proportional+monospace families so
Latin keeps the default face. No fontconfig/no CJK font -> unchanged
behaviour. Japanese track titles were observed in the historical live album
view. The public `ui-album-cjk.png` is a synthetic illustration and does not
prove that font-rendering result.

Keyboard paths verified by code inspection + compile; no synthetic key
injection harness exists — honest boundary.
