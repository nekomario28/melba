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

Melba is an unofficial, Rust-based Apple Music desktop player with its own playback daemon (`melbad`). It supports catalog browsing, a queue and native Linux media controls without an Electron shell.

## Features

- Native egui UI with queue, lyrics, MPRIS controls and 16 languages.
- Catalog search, library and playlists; AAC-256 through a local Widevine CDM, or optional ALAC through a user-supplied decrypt service.
- Direct ALSA output with format checks, plus signed portable updates for supported installations.

Lossless source audio and bit-perfect **output** are separate properties. See [Audio and playback](docs/audio.md).

## Install

Linux x86_64 and aarch64. A Rust 1.98+ toolchain and native build dependencies are required; install them first using the [distribution-specific instructions](docs/installation.md).

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

The installer creates a paired portable installation in `~/.local/opt/melba`, command links in `~/.local/bin`, and a desktop entry. The desktop starts `melbad` as needed. See [Installation](docs/installation.md) for packaging, alternate roots and updating.

For the separate, user-supplied ALAC service, see the [source-only bootstrap](docs/installation.md#optional-source-only-wrapper-bootstrap). It is external; Melba does not bundle the wrapper, rootfs or account credentials.

## Play music

Launch `melba`, choose **Sign in with Apple Music**, and select an output in Settings or with `melbad setup`. Full tracks need a paid subscription and a valid session; previews do not.

- **AAC-256 (default):** uses a Widevine CDM already present on the machine.
- **ALAC / lossless (optional):** needs a compatible decrypt service you obtain and run; otherwise playback can fall back to AAC.
- **Output:** direct `hw:X,Y`, or shared `default` / PipeWire / PulseAudio.

Sign-in and credential handling: [Authentication](docs/authentication.md). Service modes: [Wrapper control](docs/wrapper.md).

## Bit-perfect

`default`, PipeWire and PulseAudio are shared output and are never labeled bit-perfect. A direct `hw:` open that matches the source rate, channels and valid bit depth can report `BITPERFECT`.

```sh
melbad probe --json
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

Replace `hw:0,0` with your device. The verdict table, caveats and receipt format are in [Audio and playback](docs/audio.md).

Using the OS mixer instead? Its output can still pass through unchanged if Melba is the only stream, no resampling occurs and every volume is at 100%. Setup for PipeWire, and a fix for HDMI receivers that cut the start of a track when the sample rate changes, are in [Shared output (PipeWire) on Linux](docs/audio.md#shared-output-pipewire-on-linux).

## Status

Unofficial; not affiliated with or endorsed by Apple Inc. The aarch64 release is build-verified, not run-verified on arm64 hardware. Live login, track availability, output hardware and lossless playback must be checked on your configuration. See [Updating](docs/updates.md) and [Roadmap](docs/roadmap.md).

## Documentation

- [Installation](docs/installation.md) · [Audio and playback](docs/audio.md) · [Authentication](docs/authentication.md)
- [Updating](docs/updates.md) · [Integrations](docs/integrations.md) · [Development](docs/development.md)
- [Wrapper control](docs/wrapper.md) · [日本語](docs/wrapper.ja.md) · [Roadmap](docs/roadmap.md)
- [Publication scope](PUBLICATION.md) · [Third-party notices](NOTICE.md) · [Agent-facing specs](specs/README.md)

## Credits and license

The desktop frontend derives from [Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino; its original notice is retained in [`crates/melba/LICENSE`](crates/melba/LICENSE). Daemon/shared code is MIT OR Apache-2.0; the Spotifast-derived desktop crate is MIT-only. See [NOTICE.md](NOTICE.md).

Apple Music and related marks are trademarks of Apple Inc. Use of Apple's services is subject to Apple's terms. Never share account credentials, session tokens or sign-in profiles.
