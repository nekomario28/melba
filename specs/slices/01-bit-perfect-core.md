# Slice 01 — bit-perfect core

## Contract it unlocks

Audio the user owns reaches the output device at **the source's own format**:
same rate, same bit depth, same channel count, no resampler, no mixer, no volume
processing in the path. This is the claim every later slice inherits, and the
only one that can falsify the project.

## Seam

```
file → decoder (symphonia/ffmpeg) → PCM (source format) → ALSA hw / PipeWire (exclusive or fixed-format) → device
```

Owner: `src/audio/` in this repo. The seam is one function: given a decoded
stream's format, open the device in that format or fail — never negotiate down.

## What the human can run

```bash
cargo run -p melbad -- play fixtures/96k-24bit.flac --verify
# and the negative case that must NOT look like success:
pw-metadata -n settings 0 clock.force-rate 48000
cargo run -p melbad -- play fixtures/96k-24bit.flac --verify   # must refuse or report MISMATCH
```

## Verification (accept/reject on one artifact)

`evidence/slice01-format.json` must contain, per playback:

- decoded source format (rate/depth/channels) and its file provenance;
- the format actually granted by the device — `aplay --dump-hw-params` fields or
  the PipeWire node's negotiated `format`/`rate`;
- verdict `BITPERFECT` only when granted == source, `MISMATCH` otherwise, and a
  third state `CLAMPED` when the device cannot do the source rate at all
  (borrowing HiRes's device-capability rules: HDMI → ELD LPCM rates, HD-Audio →
  codec rates, else idle probe).

**Negative case is mandatory and is part of acceptance:** with the graph pinned
to 48 kHz, the core must report `MISMATCH`/`CLAMPED` and refuse to claim
bit-perfect. A player that says `BITPERFECT` while the graph resampled is the
exact bug this slice exists to catch.

Also record, once: process-group RSS at idle and during playback under a
dedicated cgroup — the \"lightweight\" number starts being measured here.

## Delegated to the implementer

Backend choice (raw ALSA vs PipeWire stream with `node.exclusive`), decoder
crate (symphonia vs shelling to ffmpeg), and internal buffer sizing. Not
delegated: the verdict vocabulary (`BITPERFECT`/`MISMATCH`/`CLAMPED`), the
requirement that a resample in the path is a failure, and the monitor-sample
authority boundary quoted in the spec README.

## Must stay green

The device stays usable afterwards: exclusive/fixed-format mode must release
cleanly, and no `clock.force-rate` is left behind
(`pw-metadata -n settings 0 clock.force-rate 0`).

## Human feedback that would change this slice

If the user's receiver turns out to be 48 kHz-only on the path they care about,
the target becomes \"fixed-rate at the device ceiling, documented\" rather than
per-track native rate — that is a product change, not an implementer call.
