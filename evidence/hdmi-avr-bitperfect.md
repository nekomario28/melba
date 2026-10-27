# HDMI / AVR bit-perfect verification

Recorded on **2026-10-27** with Melba **0.3.6** on the live portable install.

## Result

The receiver was a **Yamaha RX-V6A** on the AMD HDA HDMI output, ALSA device
`hw:0,7`. The ELD reported a present sink with LPCM support for 8 channels,
16/20/24 valid bits and 32–192 kHz. The direct PCM tests and the Apple Music
ALAC test all opened the device without an ALSA plug/mixer layer:

| Source | Granted by ALSA | Verdict |
|---|---|---|
| 96 kHz / 24-bit FLAC, 2 ch | `S32LE`, 32 valid bits (msbits), 96 kHz, 2 ch | `BITPERFECT` |
| 192 kHz / 24-bit WAV, 2 ch | `S32LE`, 32 valid bits (msbits), 192 kHz, 2 ch | `BITPERFECT` |
| Apple Music ALAC, `stream:268443097` | ALAC 192 kHz / 24-bit, 2 ch; `hw:0,7` | `BITPERFECT` |

The ALAC state advanced while playing (`positionMs` increased), and the daemon
reported `codec=alac`, `sampleRate=192000`, `bitDepth=24`, `channels=2`,
`device=hw:0,7`, `verdict=BITPERFECT`.

## Commands

For the local fixture (with PipeWire stopped so the direct device was free):

```sh
MELBA_EVIDENCE_DIR=... melbad play fixtures/96k-24bit.flac \
  --device hw:0,7 --require-bitperfect --verify --max-seconds 2
```

For the 192 kHz/24-bit check, the synthetic WAV was made without resampling:

```sh
ffmpeg -f lavfi -i 'sine=frequency=1000:sample_rate=192000:duration=1' \
  -ac 2 -c:a pcm_s24le /tmp/melba-192k-24.wav
melbad play /tmp/melba-192k-24.wav \
  --device hw:0,7 --require-bitperfect --verify --max-seconds 1
```

For Apple Music, the daemon was started with the device override and the
existing saved session:

```sh
melbad serve --device hw:0,7 --require-bitperfect --paused
melbad ctl open stream:268443097
melbad ctl play
melbad now
```

PipeWire was stopped only for the duration of these exclusive tests and then
restored. No audio configuration or credential file was changed.

## Boundary

This proves the **ALSA digital output path**: a direct `hw:` open, matching
rate/channels, enough valid sample bits, and no PipeWire/PulseAudio graph
resampler. It does not measure the AVR's analog output, volume control, room
correction or other DSP that may act after the HDMI PCM input, and it is not a
claim about every AVR, cable, EDID or track. The ELD establishes what the sink
advertised during this run.

## Local evidence

- `~/.cache/melba-hdmi-avr-20261027/receipt.json` — 96 kHz/24-bit receipt
- `~/.cache/melba-hdmi-avr-20261027/receipt-192k.json` — 192 kHz/24-bit receipt
- `~/.cache/melba-hdmi-avr-20261027/alac-now.json` — ALAC `melbad now`
- `~/.cache/melba-hdmi-avr-20261027/eld-hdmi1.txt` — RX-V6A ELD
- `~/.cache/melba-hdmi-avr-20261027/probe.json` — enumerated PCM bounds
