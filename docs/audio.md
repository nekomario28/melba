# Audio and playback

[README](../README.md) · [Installation](installation.md) · [Roadmap](roadmap.md)

This document distinguishes source encoding, decoded PCM and the actual ALSA output path. An ALAC stream is lossless **as a source format**; that alone does not establish bit-perfect output. An AAC stream remains lossy even if its decoded PCM is sent without further alteration.

## Subscription and local playback

`stream:<song-id>` plays a full catalog track when the user has an eligible Apple Music subscription and a valid `music-user-token`. Melba supports two paths:

- **AAC-256 (default):** `melba-cdm` finds a Widevine CDM already installed with a supported browser or an explicit `cdm_path` override, acquires a license using Apple's web playback endpoints, decrypts `ctrp` fMP4 segments, and passes decoding to Symphonia. This path has had live end-to-end verification.
- **ALAC / lossless (optional):** a compatible user-installed, user-run decrypt service supplies the needed audio; Melba does not ship, download or redistribute its runtime. The source may fall back to AAC when ALAC is unavailable. One missing catalog field does not establish that a track is always AAC-only.

The CDM can be configured in `~/.config/melba/config.json` with `{"cdm_path": "/path/to/libwidevinecdm.so"}`; storage is under `~/.local/share/melba/cdm-store`. The wrapper modes are documented in [Wrapper control](wrapper.md), and the licensing and redistribution boundaries in [NOTICE.md](../NOTICE.md).

Both paths use the same transport: queue commands, MPRIS `OpenUri` / `MelbaEnqueue` and `melbad ctl open/enqueue` accept `stream:` URIs. `melbad doctor` checks token validation, developer token, CDM, wrapper reachability and audio output, exiting non-zero if a necessary link fails. `preview:` URIs provide short, authentication-free catalog previews.

## Output device choice

`melbad setup` lists ALSA devices and saves `audio_device` in `~/.config/melba/config.json`. The Settings device picker provides the same choices and refresh action.

| Choice | Behaviour |
|---|---|
| `auto` | Prefer direct USB/analog `hw:` output over HDMI; never intentionally select the OS mixer, but compatibility still depends on the actual device. |
| `hw:X,Y` | Target a specific ALSA hardware PCM directly; preferred for checking exact format negotiation. |
| `default` | Follow system audio via PipeWire/PulseAudio; regarded as shared output, not bit-perfect. |

The Settings picker keeps a busy hardware output visible as “in use”. Selecting it asks PipeWire/WirePlumber to release that card through the standard D-Bus device reservation (`org.freedesktop.ReserveDevice1.RequestRelease`), then opens the direct `hw:` device. This intentionally stops other system audio on that card; a free device is not disturbed.

ALSA card numbers are assigned at boot and can change: the RX-V6A HDMI output used in the evidence below was `hw:0,7` on 2026-10-27 and `hw:1,7` on 2026-10-30 on the same machine. Check `/proc/asound/cards` (or `aplay -l`) before pinning a `hw:X,Y` value.

A 24-bit source needs a device that accepts at least 24 valid sample bits (for example an appropriate S24/S32 mode). A 16-bit-only S/PDIF interface cannot meet that requirement. For HDMI/AVR, verifying real sink capability requires a connected display/receiver and valid ELD; probing an unplugged HDMI device is insufficient.

A recorded 2026-10-27 run with a Yamaha RX-V6A is in [HDMI / AVR bit-perfect verification](../evidence/hdmi-avr-bitperfect.md); it covers 96/192 kHz PCM and a 24-bit/192 kHz ALAC stream over direct `hw:` output.

## Bit-perfect verdicts

Melba only assigns `BITPERFECT` when it opened direct `hw:` PCM, the **granted** rate and channel count match the source, and the number of valid output sample bits (`msbits`) is at least the source bit depth. It does **not** grant this label to `default`, PipeWire or PulseAudio, because a shared mixer can resample or alter PCM afterward.

| Verdict | Meaning |
|---|---|
| `BITPERFECT` | Direct `hw:` open, matching rate/channels and enough valid bits; playback proceeds. |
| `SHARED` | A non-direct device such as `default`; no bit-perfect claim. |
| `MISMATCH` | Granted rate or valid sample depth is below source requirements; exact output is refused. |
| `CLAMPED` | Exact open unavailable, e.g. S16-only S/PDIF for 24-bit content, channel mismatch, or no exact device. Negative receipt. |

`--require-bitperfect` refuses shared or mismatched output. In daemon `serve` mode it waits for a compatible device/queue change instead of silently falling back through a shared mixer.

```sh
melbad probe --json
melbad setup
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

Replace `hw:0,0` with an actual device. The file is a repository fixture and is not installed by default. `--verify` emits a JSON receipt and exits with status 0 only when the verdict is `BITPERFECT`. `MELBA_EVIDENCE_DIR` can persist receipts.

The following is an abbreviated fixture receipt from the conversion path; see [the full receipt](../evidence/slice01-format.json):

```json
{
  "source":  { "rate": 96000, "bits": 24, "channels": 2 },
  "granted": { "device": "hw:0,0", "format": "S32LE", "msbits": 32 },
  "verdict": "BITPERFECT"
}
```

The buffer depth is requested as a duration, not a fixed frame count, because the playback loop decodes and writes on one thread: the granted `buffer_frames` and `period_frames` appear in the receipt, and recovered underruns are counted (`melbad now` → `xruns`). See [the HDMI/AVR dropout investigation](../evidence/hdmi-audio-dropouts.md).

The receipt establishes the negotiated software output path reported by Melba; it is not a measurement of an arbitrary DAC's analog output. Do not infer bit-perfect from a codec label, a requested quality setting or a successful ALAC decode.

## Shared output (PipeWire) on Linux

Melba reports `default` / PipeWire as `SHARED` and never labels it bit-perfect, because it cannot see or control what the mixer does afterward. PipeWire can still pass PCM through unchanged; this is a property of the system configuration, not something Melba verifies. In practice that requires all of:

- Melba is the only active stream on that output (notification sounds, a browser or another player cause mixing).
- No resampling: the output runs at the source rate. By default PipeWire runs at 48 kHz and resamples 44.1 kHz content; allow the device rates so it switches per track instead.
- Every software volume is exactly 100% (Melba, the per-application stream, the system output). Adjust loudness on the DAC/receiver.

```
# ~/.config/pipewire/pipewire.conf.d/10-hires.conf
context.properties = {
    default.clock.rate = 48000
    default.clock.allowed-rates = [ 44100 48000 88200 96000 176400 192000 ]
}
```

PipeWire opens most devices as `S32LE`; 16/24-bit samples are placed in that container with zero padding. While playing, `pactl list sinks short` and `/proc/asound/cardN/pcmMp/sub0/hw_params` show the rate and format actually in use. Paths and syntax follow current PipeWire/WirePlumber (WirePlumber 0.5+ `.conf` files; older releases used Lua). See the [ArchWiki PipeWire troubleshooting page](https://wiki.archlinux.org/title/PipeWire/Troubleshooting) for distribution-independent background.

## HDMI receivers: track start cut off on a rate change

With shared output and per-track rates, a change such as 48 → 44.1 kHz or 44.1 → 96 kHz makes PipeWire reopen the HDMI PCM. An AV receiver then re-locks to the new format and stays muted meanwhile, so the first part of the track is lost. The PC side shows no XRun or error; the gap is in the receiver.

WirePlumber can play silence each time the output starts, before the stream's samples. The track's PCM is not modified:

```
# ~/.config/wireplumber/wireplumber.conf.d/52-hdmi-start-delay.conf
monitor.alsa.rules = [
  {
    matches = [ { node.name = "~alsa_output.pci-*hdmi*" } ]
    actions = {
      update-props = {
        api.alsa.start-delay = 96000
        session.suspend-timeout-seconds = 0
      }
    }
  }
]
```

`api.alsa.start-delay` is a count of **samples**, so the silence is shorter at higher rates: 24000 is 0.54 s at 44.1 kHz but only 0.25 s at 96 kHz. On the Yamaha RX-V6A (2026-10-30, by listening), 24000 still cut the start of a 96 kHz track; 96000 (1 s at 96 kHz, about 2.2 s at 44.1 kHz) removed it. Lower the value until the cut returns if the extra gap between tracks matters. `session.suspend-timeout-seconds = 0` keeps the output from suspending when idle, which otherwise causes the same re-lock on every resume. Apply with:

```sh
systemctl --user restart pipewire pipewire-pulse wireplumber
```


## Quality labels

The player and `melbad now` describe the format the daemon actually opened, not the format requested. AAC decoder sample precision must not be presented as AAC's *encoded bit depth*. For ALAC, sample rate, valid depth and channels come from codec-specific configuration rather than generic MP4 sample-entry placeholders. The same source format determines output negotiation; regression checks appear in `tests/source_format.rs`.

Narrow windows may abbreviate the player label; its tooltip includes the full source format, selected sink and output verdict.

## Queue and desktop control

When opening an album or playlist context, Melba replaces the queue with the original context order, selected row, initial position and play/pause intent together. Opening one track and appending others loses the preceding rows; this is not equivalent. Decoder tests under `tests/transport.rs` exercise that contract without live account data.

The daemon owns Linux MPRIS transport. The desktop launches the co-installed daemon if necessary, restoring a previous queue **paused**. Normal window close and explicit Quit stop the desktop session and playback unless background/tray mode was explicitly enabled. Switching between full and mini-player views does not quit. `melba background off` restores close-to-quit; `melba quit` performs the normal shutdown path. A standalone `melbad serve` remains possible by choice.

The implementation and installed-runtime acceptance questions are tracked in [Roadmap](roadmap.md) and [development checks](development.md); source tests alone do not prove the final physical audio or desktop behaviour.
