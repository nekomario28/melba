# Apple Music Android APK → aml-ui UX port

Source: `com.apple.android.music.apk` (split/XAPK; base 4,849 entries,
4,089 res entries). Decoded with jadx (`--no-src`) — **layout XML and
strings only; no code/assets were copied**. This is a UX-structure survey
that drove concrete aml-ui changes.

## Observed structure (layout evidence)

| APK resource | What it proves |
|---|---|
| `menu/bottom_navigation_menu_multiply.xml` | 5 tabs: Home / New / Radio / Library / Search |
| `layout/bottom_navigation.xml` | Tabs frame + `player_container` (CoordinatorLayout) — the player panel overlays content above the tab bar |
| `layout/mini_player.xml` | Mini player = thumb + 1-line title (+SharePlay line) + **play, next only**. No seek, no prev |
| `layout/player.xml` | `PlayerBottomSheetBehavior` — drag-to-expand sheet, `behavior_peekHeight` = mini strip |
| `layout/player_progress_seek_bar.xml` | Full-width MinimalSeekBar, then a row: **elapsed (left) / lossless badge pill (center) / total (right)**, monospace digits |
| `layout/constraint_player_media_controls.xml` | Under seek: centered **[prev][play][next]** — no Stop. Below: `player_lyrics` | media-route | `player_queue` |
| `layout/layout_player_buttons.xml` | Bottom of Now Playing: three equal-width toggles **shuffle / repeat / autoplay(∞)** |
| `layout/fragment_player_queue_sheet.xml`, `header_queue_{up_next,history,autoplay}` | Queue sheet with sticky sections: Now Playing → Up Next → History → Autoplay suggestions |
| `layout/empty_upnext_message.xml` + `empty_queue_title` | Centered empty state: "There's no music in the queue" + hint |
| `layout/library_main.xml`, `library_detail_page` | Library has a section picker + per-section search bar |
| `layout/lyrics_line_karaoke` / `lyrics_word_karaoke` / `lyrics_translation_line_karaoke` / `lyrics_pronunciation_*` | Lyrics sheet: line-timed karaoke; word-timed when `itunes:timing="Word"`; translation + pronunciation variants exist |

## TTML facts (live response, amp-api-edge)

- `GET https://amp-api-edge.music.apple.com/v1/catalog/{store}/songs/{id}/lyrics`
  returns `ttml` + `playParams`. **`api.music.apple.com` answers 40012
  for the same request** — lyrics switched to the edge host.
- `itunes:timing="Line"`: every `<p>` carries `begin`/`end` in
  `ss.mmm`/`m:ss.mmm`/`h:mm:ss.mmm`; `<div itunes:songPart="Verse">`
  groups lines; metadata carries `songwriters`, `leadingSilence`.
- Word-level (`timing="Word"`) tracks put `begin`/`end` on inner `<span>`s.

## What was ported (this change set)

- `aml:codec` → **Lossless pill** rendered between the seek times —
  player bar and NowPlaying both (`badge.lossless`, alac-only).
- NowPlaying view now mirrors `constraint_player_media_controls`:
  hero art → title stack → full-width seek → times+badge row →
  big prev/play/next → lyrics/queue action row → dim meta line.
  **Stop button removed** — Apple ships prev/play/next only.
- `queue_list`: centered empty state (`queue.empty` + hint).
- Karaoke: `musickit::ttml_to_lines` parses `<p begin end>` (+ timed
  `<span>` words). `lyrics_body` renders per-line: active line bright
  & larger, sung lines dim, auto-scroll centers the active line,
  **tap a line to seek** (SetPosition at line begin).
- Library: Songs/Artists/Albums section tabs; Artists/Albums list
  grouped rows, click narrows the Songs view (filter).

## Not ported (honest limits)

- Bottom-sheet drag gesture — egui has no gesture layer worth the cost;
  our panels are explicit toggles.
- Autoplay similar-music section — needs a daemon-side recommendation
  source; layout exists upstream, backend doesn't exist here.
- Recently Added shelf — needs add-date bookkeeping the scanner
  doesn't keep.
- Proprietary drawables/fonts/strings were NOT copied; labels are our
  own i18n keys.

## Verified

- `cargo test` 52 ok (new `ttml_to_lines_parses_timing` covers Line and
  Word timing, skipped bare `<p>`, bad times).
- Screenshots: NowPlaying + queue panel shows lossless pill + big
  transport + action row; lyrics panel shows live karaoke highlight on
  the final line of a playing ALAC track.
