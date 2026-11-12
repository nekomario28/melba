# What the DAC accepts, and why 24-bit playback was refused (2026-11-12)

Two questions were open: whether `hi-res` reaches the DAC at 192 kHz, and why a
24-bit source failed to open a device that advertises it.

## 1. `hi-res` does reach the DAC at 192 kHz

The daemon was run with `--device hw:0,0` (the USB DAC, 44.1–384 kHz) and the
`hi-res` setting, playing "So What" through the user's decrypt service. Read while
it played:

```
/proc/asound/card0/pcm0p/sub0/hw_params
format: S32_LE   channels: 2   rate: 192000 (192000/1)
```

So the rendition chosen by `pick_alac_variant` (24-bit/192 kHz) is what the card is
configured for. This is the DAC-side reading the earlier measurement lacked; the
earlier one read the session graph instead.

## 2. Why `24bit @ 48000Hz x2ch` was refused on `hw:0,2`

`melbad` reported:

```
open: Rejected { device: "hw:0,2", why: "no format carries 24bit @ 48000Hz x2ch" }
```

The device's own list, from `aplay -D hw:0,2` after it rejected `S32_LE`:

```
  再生中 WAVE … Signed 32 bit Little Endian, レート 48000 Hz, ステレオ
  aplay: set_params:1393: サンプルフォーマットが使用不可能
  Available formats:
  - S16_LE
  - S24_3LE
```

and a 3-byte packed 24-bit WAV plays on the same device without complaint:

```
aplay -D hw:0,2 -v /tmp/t24_48k.wav
  format       : S24_3LE
  channels     : 2
  rate         : 48000
```

`melba`'s candidate list for 24-bit content is `fmt_for_depth(24) = [S32LE, S24LE]`
(`src/audio.rs`). This device offers neither, and offers `S24_3LE`, which the list
does not contain — so every candidate is rejected and the message is accurate but
the conclusion ("the hardware cannot carry it") is wrong. The hardware carries
exactly that.

Linux has three 24-bit formats: `S32_LE` (32-bit container, 24 significant bits),
`S24_LE` (same idea, different name depending on the driver) and `S24_3LE`
(3 bytes packed). A device may offer only one of them.

### Fixed: the packed format is now a candidate, with a write path

`fmt_for_depth(24)` is `[S32LE, S24LE, S243LE]`, `fmt_msbits` reports 24 bits for it,
and both writers (`audio::play_into` and `player::play_controlled`) have a
packed branch: three bytes per sample taken from the source's top 24 bits, written
through `pcm.io_bytes()` with a frame of `3 × channels` bytes.

Measured on the device that refused before (`hw:0,2`, `lossless`, "So What"):

```
/proc/asound/card0/pcm2p/sub0/hw_params
format: S24_3LE   channels: 2   rate: 48000 (48000/1)
```

and playback progressed, 12.07 s of position after eight samples two seconds apart,
with `verdict: BITPERFECT` and `xruns: 0`.

The trap that made the first attempt fail: `Granted::format` is built with `{:?}`,
which for `alsa::pcm::Format` prints the **variant** name `S243LE` while ALSA prints
`S24_3LE`. A branch matching only `"S24_3LE"` never ran, so the stream took the i32
path and answered `io_xx: Operation not supported`. `audio::is_packed_24` accepts
both spellings and is unit-tested.

### Why the fix is not just adding the format

Adding `S24_3LE` to the candidate list was tried and measured: the device then
*opened* (stream opened once) and playback failed immediately with

```
io: ALSA function 'io_xx' failed with error 'Operation not supported (95)'
```

because the sample-writing path picks `io_i16` or `io_i32` from the granted format
and has no branch that writes 3-byte packed samples. The alsa crate exposes
`pcm.io_bytes()` for it, so the work is a third write branch in `src/audio.rs` and
`src/player.rs` (frame = 3 × channels bytes), not a one-line addition. Shipping the
format without the writer would turn a clear refusal into a misleading I/O error,
so the list was left as it was and the change is recorded here instead.

## What this does not establish

* Only one device was read (`hw:0,2`, and `hw:0,0` for the 192 kHz case); whether
  other cards offer `S24_LE` or `S32_LE` only is untested.
* The 192 kHz reading is the kernel's committed `hw_params`, not a measurement of
  the analogue output.
* No listening test was done at either rate.
