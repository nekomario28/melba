<p align="center">
  <img width="740" src="docs/assets/melba-album.png" alt="Melba album view, track list and player controls (demo library)" />
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

- **Apple Music:** catalog search, albums, library, playlists and subscription playback.
- **Audio:** AAC-256 streaming via a locally installed Widevine CDM; optional ALAC/lossless via a user-supplied decrypt service.
- **Output:** direct ALSA playback with explicit format checks, or shared system audio via PipeWire/PulseAudio.
- **Desktop:** native egui interface, queue persistence, MPRIS controls and 16 UI languages.
- **Updates:** signed portable release updates for supported installations.

Lossless source audio and bit-perfect **output** are separate properties; neither is guaranteed just by selecting a quality label. See [Audio and playback](docs/audio.md).

## Install

**Linux x86_64 and aarch64.** A working Rust toolchain (Rust 1.98+) and native build dependencies are required for the source installer. Install those first using the [distribution-specific instructions](docs/installation.md).

From a checkout:

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

The installer puts a paired portable installation in `~/.local/opt/melba` and command links in `~/.local/bin`. The desktop starts its companion daemon as needed. See [Installation](docs/installation.md) for package requirements, Arch packaging, alternate install modes and updating.

For users who also want to stage the **separate, user-supplied ALAC service**, a [source-only bootstrap](https://github.com/nekomario28/melba-wrapper-bootstrap) can install Melba from source and stage an upstream wrapper locally:

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba-wrapper-bootstrap/v1.2.3/install.sh | bash
```

**Review the external script before running it.** It requires the build dependencies above. It does not bundle the wrapper/rootfs into Melba, run `sudo`, accept terms or collect your Apple ID/password. Wrapper terms and availability remain separate; see [Wrapper control](docs/wrapper.md).

## Play music

Launch `melba`, then choose **Sign in with Apple Music**. A paid Apple Music subscription and a valid account session are required for full tracks. Catalog previews do not require a subscription.

- **AAC-256 (default):** uses a Widevine CDM already present on your machine, such as one provided by a supported Chromium-based browser.
- **ALAC / lossless (optional):** requires a compatible decrypt service you obtain and run independently; when unavailable, Melba can fall back to AAC.
- **Output device:** select one in Settings, or run `melbad setup`. Use `melbad doctor` to diagnose the playback prerequisites.

The browser and experimental native sign-in paths have different security and verification limits. Details: [Authentication](docs/authentication.md) and [Wrapper control](docs/wrapper.md).

## Audio quality

For a verified direct ALSA path, choose a compatible `hw:X,Y` device and require an exact output. `default` / PipeWire / PulseAudio are treated as shared output and are **not** labeled bit-perfect.

```sh
melbad probe --json
melbad setup
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

Replace `hw:0,0` with your device; the example needs the fixture from a source checkout and compatible hardware. The JSON receipt reports the source format, granted device format and verdict. See [Audio and playback](docs/audio.md) for definitions, caveats and evidence.

## Limitations

- This is **not an official Apple product**. It is not affiliated with or endorsed by Apple Inc.
- AAC requires a usable local CDM and authentication. ALAC additionally requires a separate service that Melba does not distribute.
- The published aarch64 release is build-verified; hardware-run acceptance is not established by that build alone.
- Live login, track availability, output hardware and lossless playback must be checked on your own configuration.

## Documentation

- [Installation and packaging](docs/installation.md)
- [Audio formats, bit-perfect verification and playback](docs/audio.md)
- [Apple Music sign-in and credential handling](docs/authentication.md)
- [Signed releases and updates](docs/updates.md)
- [Integrations, including Sidra](docs/integrations.md)
- [Development and UI acceptance tests](docs/development.md)
- [Wrapper control and troubleshooting](docs/wrapper.md) · [日本語](docs/wrapper.ja.md)
- [Roadmap and outstanding verification](docs/roadmap.md)
- [Publication scope](PUBLICATION.md) · [Third-party notices](NOTICE.md) · [Agent-facing specs](specs/README.md)

## Credits and license

Melba's desktop frontend derives from [Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino; its original notice is retained in [`crates/melba/LICENSE`](crates/melba/LICENSE).

The daemon/shared code is **MIT OR Apache-2.0**; the Spotifast-derived desktop crate is **MIT-only**. Dependencies, assets and external runtime components retain their own terms; see [NOTICE.md](NOTICE.md).

Apple Music and related marks are trademarks of Apple Inc. Use of Apple's services is subject to Apple's terms. Never share account credentials, session tokens or sign-in profiles.
