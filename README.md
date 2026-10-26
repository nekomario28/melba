<p align="center">
  <a href="#quick-start">
    <img width="740" src="docs/assets/melba-album.png" alt="Melba on Linux showing an album view with its track list and player controls, using the demo library." />
  </a><br/>
  <sub>Actual app capture · demo library · no account data</sub>
</p>

<h1 align="center">melba</h1>

<p align="center">
  <strong>Apple Music, native and fast, on Linux.</strong><br/>
  Bit-perfect ALSA playback, a real queue and library, and one signed portable install.
</p>

<p align="center">
  <a href="https://github.com/nekomario28/melba/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/nekomario28/melba/actions/workflows/ci.yml/badge.svg?branch=main" /></a>
  <a href="https://github.com/nekomario28/melba/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/nekomario28/melba?style=flat-square&label=release" /></a>
  <img alt="License: MIT / Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-2f81f7?style=flat-square" />
  <img alt="Platform: Linux x86_64 and arm64" src="https://img.shields.io/badge/platform-Linux%20x86__64%20%2F%20arm64-1f6feb?style=flat-square" />
</p>

<p align="center">
  <a href="#quick-start"><strong>Quick start</strong></a>
  &nbsp;·&nbsp;
  <a href="#bit-perfect-precisely">Bit-perfect</a>
  &nbsp;·&nbsp;
  <a href="#install">Install</a>
  &nbsp;·&nbsp;
  <a href="#documentation">Docs</a>
  &nbsp;·&nbsp;
  <a href="README.ja.md">日本語</a>
</p>

## Quick start

Melba is a Linux client; ALAC needs an Apple Music subscription and a
user-supplied local wrapper. If the build dependencies are installed, the
separate source-only bootstrap installs Melba and stages the wrapper in one
command:

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba-wrapper-bootstrap/v1.2.3/install.sh | bash
```

It installs Melba from source when absent, stages the upstream wrapper locally,
and launches the GUI. It does not run `sudo`, accept Apple terms, handle an
Apple ID/password, or redistribute the wrapper/rootfs. Build dependencies are
listed under [Install](#install). To install without launching the GUI, append
`-s -- --no-launch`.

Already have a source checkout:

```sh
sh packaging/install.sh
```

UI: 16 languages, following the system locale; see [Languages](#languages).

## Proof, not a codec label

The fixture receipt is real output from the same conversion path. It is
abbreviated; the full receipt lives in
[`evidence/slice01-format.json`](evidence/slice01-format.json).

```sh
melbad play fixtures/96k-24bit.flac --device hw:0,0 --require-bitperfect --verify
```

```json
{
  "source":  { "rate": 96000, "bits": 24, "channels": 2 },
  "granted": { "device": "hw:0,0", "format": "S32LE", "msbits": 32 },
  "verdict": "BITPERFECT"
}
```

Bit-perfect is a property of the whole output path. `default`, PipeWire and
Pulse are deliberately reported as shared; they never earn a `BITPERFECT`
label. See [Bit-perfect, precisely](#bit-perfect-precisely) for the verdict
table and the `probe` / `setup` commands.

## Audio output

`melbad setup` lists ALSA outputs and stores `audio_device` in
`~/.config/melba/config.json`. `auto` prefers a USB/analog `hw:` device over
HDMI and never selects the OS mixer; `default` follows PipeWire/Pulse and is
not bit-perfect. Use an explicit `hw:X,Y` for the exact path, and
`--require-bitperfect` to refuse shared output. The Settings page has the same
picker and can refresh the device list. A 24-bit source needs an output that
accepts S24/S32; S/PDIF interfaces that expose S16 only cannot carry it.

### Bit-perfect, precisely

Bit-perfect is a property of the **whole output path**, not of the ALAC label.
Melba claims it only when the opened device is a direct `hw:` PCM and the
kernel-granted rate and channel count equal the source, with at least as many
valid sample bits. It does not claim it for `default`, PipeWire or Pulse,
because those paths may resample or mix after ALSA opens them.

| Verdict | Meaning |
|---|---|
| `BITPERFECT` | Direct `hw:` open; rate/channels match and `msbits >= source bits`. Playback proceeds. |
| `SHARED` | The device is not a direct `hw:` path (for example `default`). Never claimed as bit-perfect. |
| `MISMATCH` | Granted rate or valid sample bits are below the source. Playback is refused as bit-perfect. |
| `CLAMPED` | An exact open was not available: for example an S16-only S/PDIF path for a 24-bit source, a channel mismatch, or no exact device. The receipt is negative. |

Check before trusting a setting:

```sh
melbad probe --json                  # devices and capability bounds
melbad setup                         # choose an output; saves audio_device
melbad play fixtures/96k-24bit.flac --device hw:3,1 \
  --require-bitperfect --verify      # JSON receipt; exit 0 only on BITPERFECT
```

`auto` prefers a USB/analog `hw:` device over HDMI and never selects the OS
mixer, but it is still a preference order, not a guarantee for an arbitrary
DAC. Use an explicit `hw:X,Y` when the exact path matters. In `serve`,
`--require-bitperfect` waits for a compatible device/queue change instead of
silently falling back through a shared mixer. The receipt records source and
granted formats; `MELBA_EVIDENCE_DIR` optionally persists it. An ALAC stream
that plays is not by itself evidence of bit-perfect output — the verdict is.
HDMI/AVR verification still depends on a connected ELD/sink.

The decrypt service for ALAC remains external: sign in through the GUI (the
runtime's native prompt when configured, otherwise the browser flow), and
Melba never downloads or bundles that service. See
[docs/wrapper.md](docs/wrapper.md) for control, stop/status and troubleshooting;
that document deliberately does not describe acquiring a runtime.

## Install

### From source

Build needs Rust 1.98 or newer, Python 3.9+, a C/C++ toolchain, CMake, `pkg-config`,
and dev headers for D-Bus, ALSA, and WebKitGTK 4.1 (used by the `auth open`
sign-in webview). Browser sign-in also needs `xdg-utils`:

- Debian/Ubuntu: `apt install build-essential cmake pkg-config libdbus-1-dev libasound2-dev libwebkit2gtk-4.1-dev xdg-utils python3`
- Fedora: `dnf install gcc-c++ cmake pkgconf-pkg-config dbus-devel alsa-lib-devel webkit2gtk4.1-devel xdg-utils python3`
- Arch: `pacman -S --needed base-devel cmake pkgconf dbus alsa-lib webkit2gtk-4.1 xdg-utils python`

The egui app itself dlopens Wayland, X11, xkbcommon and GL at run time, so
those need no dev packages.

Install the build-time license collector once (Arch's PKGBUILD declares it as
a build dependency):

```sh
cargo install cargo-about --version 0.8.4 --locked
```

The installer builds both binaries and installs a **portable** copy under
`~/.local/opt/melba` by default: `melba`, `melbad`, `melba-portable.txt`,
licenses, a desktop entry and an optional user unit. The desktop entry and
unit point at that directory; `~/.local/bin` gets stable command symlinks.
The desktop starts its companion when needed. Re-run the installer after
`git pull` to install source updates:

```
sh packaging/install.sh             # portable; add --no-unit to skip the unit
sh packaging/install.sh --cargo     # developer Cargo-root install (no GUI update)
sh packaging/install.sh --portable-dir DIR
```

The default portable layout is the shape the GUI updater accepts. Cargo-mode
installs are intentionally not GUI-updatable. The portable installer swaps
`melba` and `melbad` as one pair and restores both if their `--version` outputs
do not agree. The GUI updater does the same from the signed archive: the GUI
archive carries the companion `melbad`, and the helper restores both if the new
GUI fails to start. The installer does not enable the standalone unit or start
playback; `--no-unit` omits it entirely.

Or by hand (default Cargo paths, with `~/.cargo/bin` on your session's PATH):

```
cargo install --path . --bin melbad --locked  # the daemon (+ melba-cdm)
cargo install --path crates/melba --locked    # the melba app
cargo fetch --locked
cargo fetch --locked --manifest-path crates/melba/Cargo.toml
./ci/licenses.sh
install -d ~/.cargo/share/licenses/melba
install -m644 target/third-party/*.txt target/third-party/receipt.json \
    ~/.cargo/share/licenses/melba/
install -Dm644 crates/melba/packaging/melba.desktop \
    ~/.local/share/applications/melba.desktop
install -Dm644 crates/melba/packaging/icons/melba.svg \
    ~/.local/share/icons/hicolor/scalable/apps/melba.svg
# Optional unit for standalone daemon use; the desktop starts its co-installed
# daemon paused when needed. Do not enable an independent login player by default.
install -Dm644 packaging/melbad.service ~/.config/systemd/user/melbad.service
systemctl --user daemon-reload
```

The installer does not install a wrapper or its runtime. `melbad wrapper
status` / `melbad wrapper stop` control an already-installed user-supplied
service; see [docs/wrapper.md](docs/wrapper.md). Arch users can instead build a
real package from `packaging/PKGBUILD`
(`cd packaging && makepkg -si`; builds the remote default branch): both
binaries land in `/usr/bin`, the user unit in
`/usr/lib/systemd/user`. Enable that unit only if you explicitly want a
standalone login daemon, independent of the desktop's lifetime.

Both the installer and PKGBUILD collect source-backed notices for the two Cargo
dependency graphs and include them with the binaries. Missing or stale notices
stop installation/packaging; see [Third-party notices](NOTICE.md) for the policy,
manual regeneration, and the boundary around user-supplied runtime components.

A separate app-only Nix flake is available at `crates/melba/flake.nix`:
`nix build ./crates/melba#melba` is its package entry point, and
`nix develop ./crates/melba` opens its development shell. This route has not
been verified in the installation/licensing checks above and does not yet run
the dependency-notice collector. Use the source installer or PKGBUILD for the
notice-carrying packages described here. The Nix package installs neither
`melbad` nor its user unit.

## Apple Music sign-in

In `melba`, choose **Sign in with Apple Music** — your default browser opens
`music.apple.com`, saved passwords and MFA apply, and the app picks up the
session automatically. Firefox token capture additionally needs the
`sqlite3` CLI (`sqlite3` on Debian/Ubuntu, `sqlite` on Fedora/Arch). The
same flow is available from the CLI:

```bash
melbad auth status              # stored-token status and live validation; never the token
```

`melbad auth open` opens an embedded music.apple.com sign-in window and
captures the `media-user-token` cookie; it exposes no page-to-Rust token
IPC. Its persistent profile is private even with a permissive umask, and
existing cookie files are tightened on reuse. `melbad auth open --browser`
uses your normal browser instead: only current records belonging to
`https://music.apple.com` are candidates, not neighboring origins or deleted
LevelDB records. The browser database is copied into bounded memory and is
never opened for recovery or writes in place. IndexedDB-only sessions are not
imported: unsupported record formats are refused rather than raw-scanned; use
the embedded window or a supported localStorage/Firefox session instead.

If you run a decrypt service that exposes its own native sign-in SDK, the
sign-in screen can offer that method: Melba asks the service for its account
prompt and relays your Apple ID, password and verification code into that
session over a private local pipe — no password argument, no listener, and
nothing written to disk, logs, settings or saved UI state. The runtime
supplies the prompt text and buttons, so Melba shows the verification methods
that session offers and preserves each button's label, masked number and
position. Choosing a method only relays your choice; it is not evidence that a
message was sent. The bridge is a small C library loaded into that process and
is pinned to the payload it was inspected against; a different build fails
closed with `unsupported native authentication ABI`. This path is experimental
and is not part of the supported public configuration. Prefer `melbad auth gui`
or browser sign-in, which keep credentials out of process arguments. A failed
attempt after enrolment leaves a marker that keeps the native path out of use
until a valid sign-in completes; `melbad auth clear` removes that marker with
the saved token.

Automatic capture checks the candidate against Apple's `/v1/me/storefront`
before replacing a saved token. A readable JWT expiry is not authentication;
a failed check leaves the existing credential intact. The saved token is an
**unencrypted, owner-only file (0600)**, not an operating-system keyring.
The embedded session is kept separately under the XDG data directory's
`melba/auth-webview`; never attach either store to diagnostics. `auth clear`
removes the saved token, not the embedded session, your normal browser's
session, or Apple's server-side authorization. Manual `melbad auth set`
is an explicit unvalidated import; its token argument can enter shell history
and the process argument list.

This flow is experimental: origin filtering, database recovery, and local
storage permissions have regression coverage with fake credentials, and the
local wrapper bridge has reached the real SDK's sign-in dialog in an isolated,
credential-free probe. Credentialed native acceptance and SMS/voice delivery
remain unverified; fake SDK/validator regressions do not establish either.
Existing-token API access and playback are separate from those sign-in claims
and need live revalidation after security changes.

## Updates

**Check for updates** (in Settings → About, and the app menu) asks
`github.com/nekomario28/melba` for its latest release. Nothing is
downloaded unless that release carries `checksums.txt.sig`: a raw 64-byte
Ed25519 signature over `checksums.txt`, made with the private half of the key
committed as `crates/melba/assets/update-public-key.hex`. Dropping the key
would make the updater trust GitHub's checksum file alone, which only protects
against corruption, so the key stays in the config.

The committed public key is a verification trust anchor, not proof that a
release pipeline controls its private half. Before publishing, the release
owner must verify key provenance and control, keep the private signing key
outside the repository with restricted permissions, and test a signed artifact
against the shipped public key. With `MELBA_SIGNING_KEY` pointing to that
private file, signing is:

```bash
openssl pkeyutl -sign -rawin -inkey "$MELBA_SIGNING_KEY" \
    -in checksums.txt -out checksums.txt.sig
```

A release check and an installable update are different things. A newer
source-only tag may be reported, but installation requires signed binary
assets and a supported installation. The GUI updater requires
`melba-portable.txt` beside the executable; a copy installed under
`~/.cargo/` (including `cargo install`) is intentionally refused. Current GUI
archives carry `melbad`; the helper installs both and restores both when the
new GUI fails to start. Package-manager or source updates install both by
construction. A missing package, bad signature or failed startup must not be
reported as a successful update.

Signed v0.3.5 assets are published for **x86_64 and aarch64** from the same
source tag. The arm64 build is produced on an arm64 GitHub runner and is
build-verified, not run on arm64 hardware. The native SDK presentation bridge
is x86_64-only; arm64 uses the browser sign-in path. The optional
wrapper/rootfs remains user-supplied and architecture-specific.

The v0.3.2 → v0.3.3 portable GUI update was exercised end to end on
2026-10-26. The paired path was exercised with the v0.3.5 helper: a portable
v0.3.2 → v0.3.5 install succeeded, and a deliberately broken v0.3.7 test
release restored both `melba` and `melbad` to v0.3.6 with the receipt
"Update failed; restored the previous app". The installed `~/.cargo/bin` copy
remains a manual install, not a GUI-updatable portable installation.

Rotate or add keys with `additional_publisher_keys` in
`crates/melba/src/updates.rs`. v0.3.2 adopts the verified local publisher key
`1e47…` and keeps the old `e44c…` key as a transitional verifier. Releases
signed by the old key can still be verified by v0.3.2, but **v0.3.1 and older
do not know the new key**: they require a manual/package update before the
new signed releases. The private half of the old key is not available here, so
a transition release signed by the old key cannot be produced.

## Subscription playback (`stream:` URIs)

`stream:<song-id>` plays a full track with your Apple Music subscription.
Two flavors, chosen automatically:

- **AAC-256 (default)** — `melba-cdm` loads the Widevine CDM already on your
  machine (Chrome/Chromium/Brave/Vivaldi), takes out a license from
  Apple's `webPlayback`/`acquireWebPlaybackLicense` endpoints, and
  decrypts `ctrp` fMP4 segments in-process; Symphonia decodes and the
  normal bit-perfect ALSA path plays it. Verified end-to-end live.
  - Optional config `~/.config/melba/config.json`:
    `{"cdm_path": "/path/to/libwidevinecdm.so"}` to override discovery.
  - CDM storage state: `~/.local/share/melba/cdm-store`.
- **ALAC / lossless (optional)** — requires a decryption service that you
  supply and run yourself. This source distribution does not include,
  download or document how to obtain Apple/Android libraries, a runtime
  rootfs, keys or an APK. Without the service, playback falls back to
  AAC-256 and reports why. See
  [NOTICE.md](NOTICE.md#runtime-components-and-research-materials) for the
  runtime-component boundary and [PUBLICATION.md](PUBLICATION.md) for the
  source-only scope.

Both paths need a valid `music-user-token` (`melbad auth status`); queue,
MPRIS `OpenUri`/`MelbaEnqueue`, and `ctl open/enqueue` all accept
`stream:` URIs. `melbad doctor` checks the whole chain at once — token
(live probe), developer token, CDM presence, wrapper reachability,
audio output — and exits non-zero if any leg fails.

## Sidra hand-off (Apple's UI + melbad's engine)

`tools/sidra-bridge.py` offloads playback from
[Sidra](https://github.com/wimpysworld/sidra) — the Electron wrapper of
`music.apple.com` — to the melbad daemon. Sidra's MPRIS `mpris:trackid`
embeds the catalog song id (`/org/sidra/track/<id>`); the bridge opens
the same id in melbad (`stream:` for subscription playback, `preview:` for
auth-free 30s previews), mutes Sidra via its writable `Volume` so its UI
and progress bar stay alive, mirrors Play/Pause/Stop, forwards `Seeked`,
and re-syncs on drift >1.5 s (skipped when media lengths differ).

Verified live: `OpenUri` on Sidra raised `Playing` with the real
trackid; the bridge offloaded it and melbad reported `verdict: BITPERFECT`
on `hw:` while Sidra ran muted. Real `stream:`/ALAC hand-off needs the
user signed in on both ends — Sidra's Electron profile also yields a
`music-user-token` for `melbad auth` through the normal Chromium LevelDB
recovery path (`~/.config/Sidra/Partitions/*/Local Storage`).

```
melbad serve &
python3 tools/sidra-bridge.py --scheme stream   # or: preview
```

## Languages

The UI follows the system locale by default and can be changed in
**Settings → Language**. The choice is persisted per user. Catalogs live under
`crates/melba/assets/i18n/`:

`de-DE`, `es`, `fr`, `it`, `ja`, `ko`, `nl`, `pl`, `pt-BR`, `pt-PT`, `ru`,
`sv`, `tr`, `uk`, `zh-Hans`, `zh-Hant`

Japanese is complete at this revision; the other catalogs currently have one
or two untranslated fallback strings. Missing strings fall back to English.
Translations are gettext catalogs; `melba.pot` and `POTFILES` are the source
for updates.

### Playback quality and desktop control

The player bar's quality label and `melbad now` describe the source the daemon
actually opened, not the requested rendition. AAC is not lossless; its PCM
decoder precision is not an encoded bit depth. ALAC's encoding and bit-perfect
output are separate facts: a shared OS mixer can resample even a lossless
source. Narrow windows may abbreviate the label; its tooltip includes the
full source format, selected sink and output verdict. ALAC rate, depth and
channel count come from its codec-specific configuration, not generic MP4
sample-entry placeholders. The same source format governs output negotiation;
see the conflicting-header regression tests in `tests/source_format.rs`.

A new playback context replaces the daemon queue in its original order, with
its selected row, initial position and play/pause intent sent together. Opening
one song and appending the others is not equivalent: it loses the preceding
rows and the boundary between contexts. The private-bus decoder tests in
`tests/transport.rs` exercise this handoff without account data or hardware.

### Desktop lifetime (Linux)

The desktop starts its co-installed daemon when needed, with a restored queue
paused rather than unexpectedly playing on launch. Normal window close and
explicit Quit end the desktop session and stop playback. Keeping the app in
the tray is an explicit background preference, not the default; changing
between the main and mini-player windows is not a quit. Existing background
preferences are preserved. `melba background off` selects close-to-quit for an
existing installation; `melba quit` uses the same shutdown path as the sidebar.

The daemon owns Linux MPRIS. Window activation and application links are
forwarded to the existing desktop through CLI/IPC, with search navigation
kept distinct from track/context playback. A standalone daemon can still be
run explicitly; a desktop action cannot manufacture a missing frontend or
launch it as a child of the daemon's service group.

### Checking the UI without a display (Linux)

The windowless acceptance tests drive the real egui views with API-shaped
fixtures. An image must be fetched, decoded and referenced by a nontransparent,
clipped drawing mesh; a valid URL or a successful download alone is not enough.
Selecting a non-first album row through accessibility or pointer hit testing
must change both the player's drawn title and the load request's selected track.

```sh
cargo test --locked --manifest-path crates/melba/Cargo.toml --all-features --lib app::headless_tests
```

Each case runs with a private HOME/XDG tree and session bus, without a display,
GPU or audio playback. Credentials are synthetic. These checks establish UI
contracts, not live Apple sign-in acceptance or the final GPU/audio output.

## Project status

Release readiness, remaining hardware/legal gates and engineering debt are
tracked in [docs/roadmap.md](docs/roadmap.md). The source tree and source-only
release are usable; start with [Quick start](#quick-start).

## Documentation

- [Wrapper control and troubleshooting](docs/wrapper.md) · [日本語](docs/wrapper.ja.md)
- [Roadmap and project status](docs/roadmap.md)
- [Publication scope and boundaries](PUBLICATION.md)
- [Third-party notices](NOTICE.md)
- [Agent-native surface](specs/README.md)

## Legal / disclaimer

Unofficial, personal project. Not affiliated with, endorsed by, or sponsored
by Apple Inc. "Apple Music" and related marks are trademarks of Apple Inc.

Catalog metadata and 30-second previews are fetched through Apple's public
MusicKit web endpoints; use is subject to Apple's Terms of Service.
Subscription playback is for your own paid Apple Music account only: AAC
licenses come from Apple's license server through the Widevine CDM
installed on your machine (the same DRM module your licensed browser
uses); ALAC requires a decrypt service you supply and configure yourself.
The source-built Melba executables bundle neither Apple client libraries nor
Widevine. Any decrypt service or runtime rootfs is supplied and run by the
user outside this source distribution; its terms and redistribution are not
covered by the project's code licenses. Repository research and screenshots
also need their own rights review. See [NOTICE.md](NOTICE.md) and
[PUBLICATION.md](PUBLICATION.md).
The browser sign-in route leaves password and MFA entry to Apple's page;
melbad captures the resulting user-authorized `music-user-token` and stores
it mode 0600. Native sign-in instead accepts credentials locally and forwards them to a
user-supplied runtime; that transport is experimental and does not establish
permission to use the runtime or the Apple Music service. Do not share
credentials, the token, or sign-in profiles.

## Credits and contributions

This is an Apple Music client built on the shell and interface of
[**Spotifast**](https://github.com/crmne/spotifast), Carmine Paolino's
MIT-licensed Spotify client: the desktop crate's layout, model and egui
frontend come from it, and its original notice is kept in
`crates/melba/LICENSE` (see also `NOTICE.md`). The About card names that
project; nothing here is endorsed by it.

Pull requests are welcome, and a **better or more Apple-like interface** is
the one most worth having: the UI is the part inherited wholesale, so a
redesign that fits Apple Music's own visual language would improve this more
than any backend work. Small, focused patches with tests are easiest to take.

**Proxies** need no setting in the app: both `melba` and `melbad` read
`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` from the
environment, and there is no in-app proxy form to keep in sync.

## License

The root daemon and shared project code are licensed under MIT
(`LICENSE-MIT`) **or** Apache-2.0 (`LICENSE-APACHE`), at your option. The
Spotifast-derived desktop crate, `crates/melba`, is **MIT-only** under its
[retained original notice](crates/melba/LICENSE); the root license choice does
not relicense it. Other dependencies, assets and runtime components retain
their own terms. See [NOTICE.md](NOTICE.md).
