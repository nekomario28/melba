<p align="center">
  <img width="1000" src="docs/assets/melba-album.png" alt="Melba full window: album view, track list, sidebar and player controls (demo library)" />
  <br/><sub>Actual app capture · full demo window</sub>
</p>

<h1 align="center">melba</h1>
<p align="center"><strong>A native Apple Music client for Linux.</strong></p>
<p align="center">
  <a href="https://github.com/nekomario28/melba/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/nekomario28/melba/actions/workflows/ci.yml/badge.svg?branch=main" /></a>
  <a href="https://github.com/nekomario28/melba/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/nekomario28/melba?label=release" /></a>
  <a href="README.ja.md">日本語</a>
</p>

Melba is an unofficial, Rust-based Apple Music desktop player (`melba`) paired with its own playback daemon (`melbad`). It provides a native egui UI with catalog search, library and playlists, queue, synced lyrics, MPRIS media controls, 16 languages, and signed portable updates without an Electron shell.

## Install

Linux `x86_64` and `aarch64`. Signed portable archives (`melba` + `melbad`) are published on [Releases](https://github.com/nekomario28/melba/releases); to build and install a paired portable copy from source (requires Rust 1.98+ and the [distro build dependencies](docs/installation.md)):

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

`packaging/install.sh` installs `melba` and `melbad` under `~/.local/opt/melba`, links them in `~/.local/bin`, and adds a desktop entry. The desktop starts `melbad` on demand. See [Installation](docs/installation.md) for packaging options, alternate roots, and the [optional source-only ALAC wrapper bootstrap](docs/installation.md#optional-source-only-wrapper-bootstrap) (Melba never bundles the wrapper, rootfs, or account credentials).

## Play music

Launch `melba`, choose **Sign in with Apple Music**, and pick an output device in Settings or with `melbad setup`. Full catalog tracks require a paid subscription and a valid session; previews do not.

- **AAC-256 (default):** uses a Widevine CDM already installed on the machine.
- **ALAC / lossless (optional):** uses a compatible decrypt service you obtain and run separately, falling back to AAC when unavailable.
- **Output:** direct ALSA (`hw:X,Y`) or shared system audio (`default` / PipeWire / PulseAudio).
- **Diagnostics:** `melbad doctor` checks token validity, CDM, wrapper reachability, and audio output.

Lossless source audio and bit-perfect **output** are separate properties. Sign-in details: [Authentication](docs/authentication.md). Service modes: [Wrapper control](docs/wrapper.md).

## Bit-perfect

`default`, PipeWire, and PulseAudio are shared outputs and are never labeled bit-perfect. Only a direct `hw:` open whose granted rate, channel count, and valid sample bits match the source reports `BITPERFECT`.

```sh
melbad probe --json
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

Replace `hw:0,0` with your device. Verdict rules, caveats, and receipt format are in [Audio and playback](docs/audio.md). If you use the OS mixer instead, [Shared output (PipeWire) on Linux](docs/audio.md#shared-output-pipewire-on-linux) covers pass-through conditions (single stream, rate switching without resampling, 100% volume) and the WirePlumber start-delay fix for HDMI receivers that clip the start of a track on sample-rate changes.

## Status

Unofficial; not affiliated with or endorsed by Apple Inc. The `aarch64` release is build-verified on a native arm64 runner, not run-verified on arm64 hardware. Verify live login, track availability, output hardware, and lossless playback on your own system. See [Updating](docs/updates.md) and [Roadmap](docs/roadmap.md).

## Documentation

- [Installation](docs/installation.md) · [Audio and playback](docs/audio.md) · [Authentication](docs/authentication.md)
- [Updating](docs/updates.md) · [Integrations](docs/integrations.md) · [Development](docs/development.md)
- [Wrapper control](docs/wrapper.md) · [日本語](docs/wrapper.ja.md) · [Roadmap](docs/roadmap.md)
- [Publication scope](PUBLICATION.md) · [Third-party notices](NOTICE.md) · [Agent-facing specs](specs/README.md)

## Credits and license

The desktop frontend derives from [Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino; its original notice is retained in [`crates/melba/LICENSE`](crates/melba/LICENSE). Daemon and shared code are MIT OR Apache-2.0; the Spotifast-derived desktop crate is MIT-only. See [NOTICE.md](NOTICE.md).

Apple Music and related marks are trademarks of Apple Inc. Use of Apple's services is subject to Apple's terms. Never share account credentials, session tokens, or sign-in profiles.
