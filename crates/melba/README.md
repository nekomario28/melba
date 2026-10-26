<p align="center">
  <img src="packaging/icons/melba.svg" alt="Melba logo" width="88" height="88">
</p>

<h1 align="center">Melba</h1>

<p align="center"><strong>Apple Music, native and fast.</strong><br>A lightweight Apple Music app for Linux.</p>

Melba is an Apple Music client written in Rust with
[egui](https://www.egui.rs) and [fastframe](https://fastframe.dev). It talks
to Apple's MusicKit API for the catalog and library, and plays through the
`melbad` daemon in this repository — including lossless ALAC where your
subscription and the available streams allow it. No browser engine, no
Electron.

**Playback needs an Apple Music subscription.** Browsing and search work
without one; playback does not.

| Feature | What you can do |
|---|---|
| **Library and search** | Browse your playlists, favorites, albums, artists, and Apple Music's catalog. Like songs and albums, create playlists, add and remove tracks. |
| **Home and recommendations** | Heavy rotation, recently played, and personal station picks on the Home shelf. |
| **Playback** | Play locally through the `melbad` daemon, seek, shuffle, repeat, and manage a persistent queue. |
| **Lyrics** | Follow time-synced lyrics where Apple provides them. |
| **Themes** | Light, dark, system appearance, or custom colours. On Omarchy, follow your desktop theme. |
| **Desktop controls** | MPRIS media keys, keyboard shortcuts, a tray icon, and a `melba <verb>` command line. |

## Install

Melba builds from this repository:

```sh
cargo install --path crates/melba
```

Playback and sign-in need the `melbad` daemon, built from the repository root:

```sh
cargo install --path .
```

## Start listening

1. Open Melba and choose **Sign in with Apple Music**. Your default browser
   opens `music.apple.com` — saved passwords and MFA work there. When the
   browser finishes, Melba picks up the session automatically.
2. Pick a song. The `melbad` daemon handles playback on this computer.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. To look at the interface
without an Apple Music account, run:

```sh
cargo run --features demo -- --demo
```

Translations live in `assets/i18n/`; release packaging is described in
[PACKAGING.md](PACKAGING.md).

## Acknowledgements

Melba's interface began as a port of
[Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino (MIT),
adapted from Spotify to Apple Music. It uses
[egui](https://www.egui.rs), [fastframe](https://fastframe.dev),
[Symphonia](https://github.com/pdeljanov/Symphonia), the
[Inter](https://rsms.me/inter/) typeface (OFL), and
[Lucide](https://lucide.dev) icons (ISC).

Melba is an independent project and is not affiliated with Apple.
Apple Music is a trademark of Apple Inc.

Licensed under the [MIT License](LICENSE).
