# ALAC bit-perfect + MPRIS media-control completeness — 2026-10-15

## ALAC: bit-perfect claim and proof chain

Claim: local ALAC playback is bit-perfect end-to-end **at the ALSA boundary**.

| layer | mechanism | evidence |
|---|---|---|
| decode | symphonia `alac` → interleaved full-scale `i32` (`audio::decoded_to_i32`, shared by `player::play_controlled`) | `tests/bitexact.rs`: **byte-identical** to `ffmpeg -f s32le` reference for 16-bit (`88k-16bit.m4a`) and 24-bit (`96k-24bit.m4a`) ALAC, plus FLAC 16/24 controls. Length delta ≤ one packet allowed for container priming; observed delta = 0. |
| sink write | `write_all_i32`/`write_all_i16` do verbatim `writei`; S16 path is `i32>>16` | `s16_sink_path_is_exact_for_16bit_sources` compares `>>16` output to `ffmpeg -f s16le` — identical. |
| ALSA grant | `hw:` exclusive at source rate/channels, `msbits ≥ bits` | `ci/check.sh --hardware` hires.bitperfect PASS; live run on `hw:0,0`: ALAC 96k/24-bit verdict `BITPERFECT`. |

What it does NOT prove: post-kernel DAC path (internal to the DAC), and formats
played through a non-exact grant still report CLAMPED/MISMATCH honestly.

## MPRIS / Linux media-control completeness

Now exposed on `org.mpris.MediaPlayer2.aml`:

- `mpris:trackid` — unique per play (`…/aml/Track/{seq}`), controllers detect track changes.
- `xesam:title`, `xesam:artist` (`as` array), `xesam:album`, `xesam:url` (absolute `file://`),
  `mpris:length`, `mpris:artUrl`.
- Album art: catalog `apple.artwork` URL for preview/collection tracks; embedded
  cover extracted to `~/.cache/aml/art/<sha>.<ext>` (`file://`) for local files;
  cleared to `""` for tracks without art.
- Signals: `PropertiesChanged{PlaybackStatus,Metadata}` emitted from the serve
  loop on any state/track change (incl. auto-advance); `Seeked` emitted by
  Seek/SetPosition; `LoopStatus`/`Shuffle` auto-emit on `Set`
  (`emits_changed_true`); `Position` declared `emits_changed_false` per spec.
- Methods: Play, Pause, PlayPause, Stop, Next, Previous, Seek, SetPosition,
  OpenUri (file:// + preview:), Quit, Raise(noop), `AmlGoTo` (queue jump).
- Capabilities: CanPlay/Pause/Seek/Control true; CanGoNext computed from
  queue+repeat+shuffle; CanGoPrevious = non-empty queue; CanQuit true;
  DesktopEntry=aml; SupportedUriSchemes=file+preview; MIMEs = flac/m4a/mp3/ogg/wav/caf.
- `aml now` JSON gained artist/album/artUrl/uri fields.

Runtime verification (busctl monitor capture): 10 `PropertiesChanged` and a
`Seeked` signal observed across start/pause/play/seek/prev; `ctl quit` exits
the daemon cleanly; `ctl open preview:` at end-of-queue revives playback with
full Karajan-9 metadata + mzstatic artwork.

## Bugs found and fixed this pass

1. **ABBA deadlock** — `advance()` takes `idx→pending_idx→queue`; readers held
   `queue` while locking `idx`. Every read is now sequential (copy, drop, next).
2. **Queue-end wedge** — playback thread held the `queue` guard across
   `rx.recv()` when the queue emptied → daemon stopped answering all D-Bus
   calls at end-of-queue. Guard now dropped before `recv`.
3. **Prev self-deadlock** — `*idx.lock() = idx.lock()…` locked a non-reentrant
   mutex twice in one statement (×2 sites). Would hang `ctl previous` forever.
4. **Prev wrong semantics** — decremented `idx` then `advance()` re-incremented
   → always replayed the same track. Now uses `pending_idx` like `AmlGoTo`.
5. **MP4/m4a tags never read** — `probed.metadata.get()` is empty for MP4;
   fixed to `probed.format.metadata()` (also repairs scan_dir titles for m4a).
6. **`mpris:trackid` was static** `/Track/current` — media widgets saw no track
   change. Now `/aml/Track/{seq}`.
7. **Root `Quit` method missing** though `CanQuit=true` and `ctl quit` called it
   → always UnknownMethod. Registered (sets quit flag + Cmd::Quit).
8. **`xesam:url` relative** — serve canonicalizes queue paths to absolute.

## NOT proven here

- KDE/Wayland widget or `playerctl` rendering of the artwork (no such client on
  this host during the pass; wire-level metadata + signals verified instead).
- Bridge (`applemusic-web`) parity: same verbs exist; artUrl projection already
  present from the earlier pass.
