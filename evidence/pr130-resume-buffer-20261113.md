# PR #130 — native Resume failure and buffered position

Follow-up to the partial [RX-V6A acceptance](pr130-hardware-20261113.md),
2026-11-13 12:03:31–12:03:33 JST. Same physical `hw:1,7` Yamaha RX-V6A.

## Mechanism and correction

A failed Resume can invalidate PCM pointer/delay accounting. The player now
captures the frozen ring's unplayed frames (`buffer_frames - avail`) immediately
following successful native Pause, before trying Resume. If Resume fails, the
captured audible position is parked before dropping/closing the old handle.
Running Pause failure likewise uses a position captured before the attempted
Pause. Fractional milliseconds are subtracted in frame/rate units before
flooring, matching the existing elapsed-position convention.

Paused Seek/restart discards the old buffer and parks the requested cursor;
old samples must not be subtracted from a new decoder position. If buffer
accounting is unavailable or out of bounds, failed Resume stops with a visible
`pause position unavailable` verdict instead of silently assuming no tail.

## Red → green regression

The ALSA fault fixture models an unplayed 9600-frame tail (200 ms at 48 kHz),
then drops the old PCM when rejecting Resume. The old player reopened at
528 ms instead of the expected 328 ms. The revised player passes two cycles,
requires the buffer-snapshot and failed-Resume traces, and requires a new stream
open. An invalid available-space result also tests the explicit stopped/error
path. Integer-frame tests cover fractional positions at 44.1/48/96 kHz.

## Physical device, injected Resume failure

For this probe only, `tests/fixtures/pcm_pause.c` rejected Resume and invalidated
the old PCM. No ring sizes, available-space values, hardware positions or file
seeks were replaced. Optional fixture instrumentation observed real
`snd_pcm_avail`, `snd_pcm_delay`, and the WAV decoder's actual `lseek64`.
The 45-second low-amplitude 48 kHz/24-bit WAV was played in an isolated D-Bus/XDG
session; the target PipeWire sink was restored via exit/signal cleanup.

| Cycle | Submitted position | Buffer | Available | Unplayed | Saved target | Actual coarse WAV seek | Opens | XRuns |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1032 ms | 24576 frames | 3718 frames | 20858 frames | 597 ms | 576 ms | 2 | 0 |
| 2 | 1389 ms | 24576 frames | 2949 frames | 21627 frames | 938 ms | 936 ms | 3 | 0 |

After reopening, reported position progressed to 1365/1706 ms respectively.
The ring snapshots matched the kernel application/hardware pointer difference.
The saved targets account for that actual tail instead of jumping past it.

**Important evidence correction:** the PAUSED `/proc` status reported delay=0,
but `snd_pcm_delay()` on that same paused PCM reported 20858/21627 frames,
matching the actual ring tail. The earlier report's zero kernel-status field
was not proof that the Rust `pcm.delay()` API itself returned zero. This probe
separates those quantities. The fix protects the post-failure invalidation
case; it does not attribute a spontaneous driver failure to this receiver.

## Validation and boundaries

- Rustfmt, Clippy with warnings denied, all daemon/workspace Rust tests, the
  material-provenance tests, and the rebuilt daemon passed locally.
- Probe binary SHA-256:
  `878354b9281dbf2b9d9fd432025e5c2725eadb722f84b31533efcf03e2672b06`.
  Parent revision: `1db4d34f5be115d75a620e951bf4d83c6891b10f`.
- The real ring-buffer/reopen path was exercised; the Resume failure was
  **injected**, not a naturally observed unplug/reconfiguration failure.
- Existing Symphonia coarse seeking lands on a packet boundary. The measured
  decoder starts were 21/2 ms before the saved targets (under one 24 ms WAV
  packet), not sample-accurate seeks. Other codecs retain their own coarse-seek
  behavior; this is not a promise of sample-exact resume.
- No analog waveform or audible continuity measurement was made. User listening
  acceptance remains required before merging. No global PipeWire stop, account
  operation, normal queue/configuration write, or release change was performed.

Raw timestamped results, ring snapshots, fault traces and the reproducible
probe scripts are retained locally in the dedicated Resume-validation folder;
this report omits personal paths and session material.
