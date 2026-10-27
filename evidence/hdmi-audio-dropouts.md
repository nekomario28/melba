# HDMI/AVR playback gaps: cause and fix

Recorded on **2026-10-28** on the live portable install. Symptom: playback on
`hw:0,7` (Yamaha RX-V6A over the AMD Navi 31 HDMI output) pauses or goes
silent, then continues.

## Ruled out: the HDA FIFO error

The journal shows 140 messages from the HDMI controller while playing:

```
snd_hda_intel 0000:03:00.1: stream 0 dma error: 0x28
```

`0x28` is the HDA stream status register: `SD_INT_FIFO_ERR (0x08)` plus
`SD_STS_FIFO_READY (0x20)`, which is why `snd-hda-core` warns
(`sound/hda/core/controller.c`).

This is **not** Melba and **not** the cause of the gap:

| Test (20 s of 44.1 kHz/16-bit stereo) | FIFO warnings |
|---|---|
| `aplay -D hw:0,7`, default buffers | 40 |
| `aplay -D hw:0,7 --period-size=4096 --buffer-size=16384` | 40 |
| `melbad play --device hw:0,7` (same buffers) | 40 |
| `aplay -D hw:3,1` (USB, control) | 0 |

Every HDMI PCM of that controller reproduces it; the USB device does not. The
count is capped by `dev_warn_ratelimited` (10 per 5 s), so 40 per 20 s is that
ceiling, not the true rate. A 20-second poll of the same stream showed the
stream never stalling: `state=RUNNING` throughout, `delay` constant at
16383/16384 frames, `stalls=0 recovers=0`, and written frames equal to
elapsed × rate. The warnings are a property of this controller, not of the
player.

## Cause: the buffer was a frame count, not a duration

`open_exact` asked for `buffer_size_near(4096 * 4)` = 16384 frames, so the
slack the writer had was a different amount of time at every rate. Measured by
stalling a writer with the same hw params and watching for the first XRun:

| Source rate | Buffer | Stall that caused an XRun |
|---|---:|---:|
| 44.1 kHz | 371 ms | 300 ms |
| 96 kHz | 171 ms | 150 ms |
| **192 kHz** | **85 ms** | **80 ms** |

The playback loop decodes and writes on one thread, and for a `stream:`
source each `next_packet()` may wait on the network and the managed decrypt
service. At 192 kHz — the rate of the Apple Music ALAC used here — a
**80 ms** stall was enough to drain the device, which then produced an
audible gap and an automatic recovery.

A second defect made this invisible: both write paths recovered an XRun with
`try_recover` and carried on **without any counter or log**. A gap Melba
caused could not be attributed.

The session where the user reported the symptom also shows the daemon's
cgroup with 55.5 MB swapped while several multi-gigabyte test scopes and a JVM
ran, which is the kind of scheduling stall the small buffer cannot absorb.

## Fix

- The buffer is requested as a **duration** (`BUFFER_MS = 500`, never below
  four periods) and the granted `buffer_frames`/`period_frames` are recorded
  in the `--verify` receipt, so a regression is visible in evidence.
- Recovered XRuns are **counted and published**: `melba:xruns` in
  `melbad now`, the first one per track on stderr, and an
  `xrun_recoveries=N` note in the receipt.

Measured after the fix on the same controller:

| Rate | Buffer before | Buffer after |
|---|---:|---:|
| 44.1 kHz | 16384 (371 ms) | 20480 (464 ms) |
| 96 kHz | 16384 (171 ms) | 49152 (512 ms) |
| 192 kHz | 16384 (85 ms) | 94208 (491 ms) |

The 192 kHz stall tolerance moved from ~80 ms to 200–300 ms in the same test.

## Boundary

This is the ALSA digital output path: buffer depth, decode/network scheduling
and recovery visibility. It does not measure the receiver's own behaviour —
input switching, standby, room correction or other DSP after the HDMI PCM
input — and a gap caused by the display or receiver renegotiating the link is
outside this fix. While testing, `melbad serve --paused` was observed holding
`hw:0,7` exclusively with playback paused, so the rest of the desktop cannot
use that HDMI output meanwhile; that is a separate design question and was not
changed here.
