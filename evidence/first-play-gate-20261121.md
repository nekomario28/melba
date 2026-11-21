# First-play gate measurement, 2026-11-21

Scope: the playback-side items of the roadmap's *Authentication and first-play
release gate*, measured on the maintainer's account against **melbad 0.4.0**
(installed release, `~/.local/bin/melbad`). The sign-in-side items (first
sign-in, 2FA delivery choices, GUI sign-out/re-login on the live install)
remain untested — they need an interactive credentialed session.

## Method: isolated daemon, live credential copy

Each run spawned a second `melbad serve --device null` on a private
`dbus-run-session` bus with private `XDG_CONFIG_HOME`/`CACHE`/`DATA`/`STATE`/
`RUNTIME_DIR` and a private `ALSA_CONFIG_PATH` (`pcm.null { type null }`), the
same recipe `tests/transport.rs` uses. The live install's daemon, queue, and
files were untouched. The only shared object was a **read** of the stored
`music-user-token`, copied mode-0600 into the isolated config dir and deleted
with the scratch tree; `cdm_path` pointed at the existing Vivaldi CDM;
`audio_quality` was `aac`.

## Measured

- **AAC full-track playback past 30 s — PASS.** `ctl open stream:1679278167`
  (YOASOBI — アイドル, 213 281 ms) on the isolated daemon: metadata resolved,
  license+probe took ~16 s, then `status=Playing`, `codec=mp4a@44100`,
  `verdict=SHARED` (null is not an exclusive claim, as designed). Position was
  observed at 13 397 ms then **121 022 ms** before the track completed to
  `Stopped` — past the 30 s preview boundary. The null sink is unpaced: the
  whole track wrote in a fraction of a second, so segment fetch+decrypt had
  largely already happened during the open; that is why the Playing window is
  only a couple of polls wide.
- **Decrypt pipeline independently verified.** `melba-cdm`'s `e2e` binary
  (manifest → Widevine cert → license → decrypt 3 fragments) ran live:
  `usable_key=true`, 1 290 frames, 995 723 clear bytes.
  `melbad play` of that output wrote 1 320 960 frames to the null sink.
- **Saved-session restore — PASS (isolated).** `ctl quit` then `serve`
  restored `XDG_STATE_HOME/melba/queue.json` ("restored queue" logged,
  file persisted).
- **Sign-out path — PASS (isolated copy).** `melbad auth clear` removed the
  copied token; `auth status` reports "no token stored".
- **Rejected token — PASS.** A bogus token reported `INVALID (token rejected:
  http status: 403)` through live validation — a bad credential is detected,
  not trusted on file presence.

## Interpretation notes

- `melbad: level=debug event=next_packet_failed reason="end of stream"` fires
  at normal track end on this sink — it is not a failure signature by itself.
- `verdict=SHARED` on `null` is correct: only a device whose granted name
  starts with `hw:` can earn `BITPERFECT`; `null`, `plughw:`, `default` and
  every other name are `SHARED` by construction (`src/audio.rs`).
- An earlier poll rate (4–5 s) made completed playback look like an instant
  failure: the null sink finishes a prefetched track inside one poll gap.
  Position accounting (pos_ms reaching 121 s mid-flight) is what proves the
  path executed — CV1 applies to this measurement.
- One 75 s run showed the open call wedged somewhere in the source-open path
  (no verdict change, `streamOpens` stayed 0) — consistent with a stalled
  HTTP call without a total timeout; a retry completed normally. Not
  characterized further here.

## Still open (needs the maintainer or a consenting test account)

- First sign-in (embedded, browser, and the experimental native SDK path),
  including which verification-delivery methods Apple actually offers.
- GUI sign-out → re-login on the live install (destructive to the session).
- Real expiry/rejection recovery while signed in (not a planted bad token).
- Credentialed run against a disposable account (unified-signin Gate 1).
- Physical checks: hi-res DAC reading, aarch64 hardware.
