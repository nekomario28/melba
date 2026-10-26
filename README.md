# melba — Apple Music, native and fast, on Linux

Native Linux Apple Music client in Rust: bit-perfect local hi-res playback
over ALSA `hw:`, catalog metadata via public MusicKit endpoints, preview
streaming, and full-track subscription playback — AAC-256 decoded in-process
through your installed Widevine CDM (`crates/melba-cdm`), ALAC lossless
through an optional user-supplied local decrypt service.

The project has two binaries:

- **`melba`** — the desktop app (`crates/melba`): Home, Browse, Search,
  Library, playlists, lyrics, queue, MPRIS media controls, a tray icon,
  themes, and a `melba <verb>` command line. This is the product you open.
- **`melbad`** — the playback daemon (repository root): the ALSA playback
  engine, the MusicKit/streaming plumbing, and the MPRIS endpoint the UI
  talks to.

Docs, spec, feature matrix, and agent-native surface: `specs/README.md`.

```
melbad probe [--json]            # PCM devices + capability bounds
melbad setup                     # pick an output and save audio_device
melbad config audio-device hw:3,3 # or auto/default
melbad play <file> --require-bitperfect --verify  # refuse the OS mixer; receipt JSON
melbad serve <files|collection>  # daemon + MPRIS; honours audio_device from config
melbad ctl / melbad now / melbad about   # transport + JSON state + manifest
melbad mk search "<q>" --collection | melbad serve /dev/stdin
melbad ctl open stream:1490256995        # full-track subscription stream
```

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
melbad play fixtures/96k-24bit.flac --device hw:3,1   --require-bitperfect --verify      # JSON receipt; exit 0 only on BITPERFECT
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

The installer builds both binaries, installs the desktop files, and registers
an optional standalone user daemon unit without enabling or starting it.
The desktop starts its companion when needed (re-run the installer after
`git pull` to install source updates):

```
sh packaging/install.sh            # add --no-unit to skip systemd setup
```

It installs under `CARGO_INSTALL_ROOT` (default: `CARGO_HOME` or `~/.cargo`)
and records absolute executable paths, so desktop launch and the daemon do
not depend on Cargo's bin directory being on your session's PATH. For
sign-in, the GUI prefers the daemon beside its own executable and falls
back to PATH only when it is installed separately. `--no-unit` skips the
standalone systemd unit; the desktop can still start its companion directly.

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
assets and a supported installation. The current updater replaces the GUI,
not its companion daemon: a GUI-only replacement is refused unless the
co-installed `melbad` already matches the target version. Package-manager or
source updates must install both together. A complete paired-package
self-update with rollback remains a release gate; an unavailable package or a
failed check must not be reported as a successful update.

Rotate or add keys with `additional_publisher_keys` in
`crates/melba/src/updates.rs` (ship the next key first, keep signing with the
current one, then switch).

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

## i18n

`melba` ships complete translations under `crates/melba/assets/i18n/`
(16+ languages, gettext catalogs); it follows the system locale.

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

## Roadmap / TODO

### Release readiness priorities

Prioritize a reliable native Melba release over recreating the official UI.
The next release should make audio preferences effective, playback dependable,
and the actual playback path understandable.

- [ ] **Audio quality settings** — add AAC high-quality, lossless and hi-res
  lossless preferences, plus an explicit policy for allowing AAC fallback or
  requiring lossless. Select renditions using their verified format, not their
  order in a playlist. The daemon owns these preferences; GUI and CLI use the
  same API. Verify application, persistence and restoration after restart,
  rather than only saving a control's displayed value.
- [ ] **Track-selection and queue acceptance** — verify selection of a non-first
  row in an album/playlist during playback on the installed app. Context
  resolution must retain its offset, and a late older result must not undo a
  newer play. Verify the resulting queue and Next/Previous order, not just the
  selected title. Follow the [runtime verification policy](crates/melba/AGENTS.md#runtime-verification).
- [~] **Lossless (ALAC) availability** — playback can open ALAC through a
  user-supplied decrypt service, but separate catalog probes and earlier playback have also
  yielded AAC. Explain availability and fallback at the actual source-opening
  boundary; neither one missing `enhancedHls` field nor one successful ALAC
  stream establishes what every track or account can play. Cover failures in
  probing/initial decoding as well as acquisition. A synthetic multi-run
  regression now covers the `subsample overruns sample` failure in
  `crates/melba-cdm/src/stream.rs`: sample runs sharing an encryption table
  must retain sample order. Bounds remain strict; installed-runtime acceptance
  of the affected track is still required.
- [ ] **Quality details** — add the encoded bitrate and a user-facing fallback
  reason when reliably known. Do not infer an exact bitrate or declare a
  track permanently AAC-only from one missing catalog field.
- [ ] **Desktop lifecycle and MPRIS acceptance** — validate the installed
  close/Quit/startup path and daemon-owned window/search/context forwarding.
  Explicit background mode may keep playback alive; ordinary close should
  not. Source implementation and private fixtures are tracked in
  [the desktop lifecycle plan](specs/desktop-lifecycle-update/plan.md), not a
  claim that the installed client has passed acceptance.
- [ ] **Paired signed self-update** — publish installable frontend/daemon
  packages and prove replacement, rollback and restart as one transaction.
  A real release check alone is not a working whole-client installer.
- [x] **Choose the output from the app, over PipeWire** — `audio_device` in
  `~/.config/melba/config.json` selects `auto`, `default` or `hw:X,Y`;
  `auto` prefers a USB/analog hw device over HDMI, the Settings page lists
  outputs and saves the choice, and `--require-bitperfect` refuses `default`.
  Still open: following the system default automatically and showing the
  opened rate/verdict beside the picker.
- [~] **First-run audio setup** — `melbad setup` and the Settings picker work;
  a guided first-launch wizard remains.

- [ ] **HDMI/AVR bit-perfect verification** — the HDMI PCM devices expose
  S16_LE/S32_LE at up to 192 kHz, but this host currently reports no HDMI ELD
  or sink. When an AVR/display is connected and selected, confirm
  `monitor_present`/`eld_valid`, then run
  `melbad play <track> --device hw:X,Y --require-bitperfect --verify`; persist
  `audio_device` only after a `BITPERFECT` receipt.
- [ ] **Authentication release gate** — verify first sign-in, saved-session
  restore, rejected/expired sessions and sign-out. Keep diagnostic credentials
  and SDK state isolated from the live account. Fake peers do not establish
  real SDK acceptance or SMS/voice delivery.
- [ ] **Release audit and packaging** — review the complete pending changes,
  finish license/credit and secret-handling checks, resolve test isolation,
  and run the affected GUI, daemon and packaging gates on the release inputs.
  Verify the intended installable artifacts and their checksums before claiming
  binary-release readiness.

Keep requested quality, opened-source format and output-path properties
separate. Do not expose controls for processing the daemon does not implement,
or infer bit-perfect playback from an ALAC label.

### Publication and wrapper packaging

- [x] **Keep the project's MIT/Apache-2.0 terms.** `NOASSERTION` was rejected:
  removing a license does not reduce DRM or contract exposure, drops the
  patent grant, and makes use and redistribution unclear while dependency
  notices (MIT/MPL/Apache) remain. Risk is managed by scope, not by removing
  the license.
- [ ] **Download/export stays outside Melba.** Saving full tracks is a
  separate product with its own legal review; Melba remains playback-only.
- [ ] **Wrapper in the standard install (ideal, blocked).** The best
  experience would be: install Melba, sign in, ALAC works. That requires
  distributing a runtime bundle and rights not established here; the public
  source must not contain or fetch Apple/Android libraries, a rootfs or an
  APK. Until then the supported path is a user-supplied decrypt service.
  Options: guided setup for a user-provided service, an officially permitted
  SDK, or a separate service project with its own review. "Bundled" is not
  approved. Bundling would not change the DRM/contract question, but it adds a
  separate redistribution act for Apple/Android libraries; that extra act is
  why it stays blocked even when the DRM risk is accepted.
- [ ] **Public export scope.** Decide whether managed-mode code and runtime
  staging scripts are excluded or shipped disabled; decide
  history-inclusive repository vs current-tree export; complete the legal
  review before any push, release or visibility change.

### Deferred research and presentation

- [ ] **Official Apple Music reference via WinBoat** — assess installation and
  execution of the Windows client in a resource-budgeted, isolated VM. Use it
  to investigate quality-setting semantics, rendition selection, fallback and
  error behavior. Treat the proprietary client as `REFERENCE_ONLY`: document
  behavior for an independent Melba implementation, not a redistributable fork
  of Apple's code or assets. Minimize host sharing and keep live credentials
  out of captures. RDP audio is not a bit-perfect reference. Background:
  [WinBoat architecture](https://github.com/TibixDev/winboat#how-it-works) and
  [Apple's lossless guide](https://support.apple.com/en-us/118295).
- [ ] **Optional presentation and DSP work** — consider an official-inspired
  selectable style, EQ, loudness normalization/Sound Check-like behavior,
  crossfade and Atmos separately after the release-critical work. These are
  deferred candidates, not implemented capabilities or release prerequisites.

### Product gaps

- [x] **URL-scheme handoff restored** — `melba` accepts `musics://`,
  `itms://`, `itmss://`, `music://`, `apple-music://` and
  `music.apple.com` links (`melba <link>`); `melba.desktop` registers the
  handlers with `%u`, and a running instance receives forwarded links.
- [x] **Real account name in the UI** — `ApiRequest::Me` now reads
  `/v1/me`'s social profile (id + display name); settings and the topbar
  show the real account instead of a fixed "Apple Music".
- [ ] **Release binaries** — releases are source-only today; no artifact
  pipeline, checksums, or download page. Decide whether GitHub release
  artifacts or a native-package pipeline (`packaging/native-packages.yaml`,
  untested) is worth the maintenance.

### Engineering debt

- [x] **librespot dead code removed** — the inherited Spotify engine
  (`backend.rs`'s old Worker, `player.rs` internals, `zeroconf`, `sonos`,
  `cast`, `sink`, `auth`, `credentials`, `prerelease`, `resample`,
  `session_reads`, `made_for_you`, `api/client`, `api/gateway`,
  `ui/prerelease`) is gone; the daemon worker is the only backend path.
- [x] **Workflow commit** — the `.github/workflows/ci.yml`
  `AML_BIN`→`MELBA_BIN` fix is pushed (`e1fd35c`). HTTPS pushes carrying
  workflow edits are rejected without the `workflow` OAuth scope; the
  user's own SSH remote is not subject to that token restriction, which
  is the route that shipped it.
- [x] **Dead ApiRequest surface removed** — Apple Music has no podcasts,
  so the Spotify-era show/episode chain was cut end to end: the six
  send-only request variants (`Show`/`ShowEpisodes`/`Episode`/
  `HomeEpisodes`/`SavedShows`/`SavedEpisodes`), their reply variants,
  the never-sent `SearchCatalogue`/`SearchPlaylists` split-search pair
  and the `SearchStarted` reply, `Page::{Show,Podcasts,Episodes}` and the
  whole UI chain (show view, library shelves, search filters and cards,
  home podcast shelf, settings legacy value kept for old files), plus the
  split-search state (`search.playlists`, `playlists_pending`,
  `show_search_playlists`). `ci/models/daemon_api.py` now pins
  UNANSWERED/DEAD_RESPONSES as empty sets — the drift guard fails if a
  new send-only variant appears. (`Remote`/`Transfer`/`ShufflePlay`
  vestigial sends were cut in `7cb12d6`.)
- [x] **Foreign-device surface removed** — `melbad` only ever lists this
  computer, so the Connect-style machinery was unreachable: the devices
  popup and its topbar badge, `selected_device`, `Action::Transfer`, the
  `transfer` control verb, `PlayerCommand::Transfer` (a daemon no-op),
  and `local_transfer_sequence` are gone. `Target::Remote` is now a unit
  variant meaning "no ready playback surface" — it still queues play
  requests while the daemon connects. The external `devices` verb stays
  and answers the honest local-only list (self-refreshing on read).
- [x] **Producer/consumer sweep done** — a systematic pass over enums,
  fields, settings keys, deps and `allow(dead_code)` helpers found and
  fixed a real break: `ApiRequest::Me` had lost its only send site in
  the engine removal, so account profile/product never populated in
  production (restored in `handle_auth`, pinned by a regression test).
  Also removed: `Action::EnablePlayback`/`RefreshQueue`,
  `Command::VerifyResume`/`RadioResolved`, write-only `sign_in_url`,
  five unread settings keys (`bitrate`, `audio_buffer_ms`,
  `audio_cache`, `audio_cache_mb`, `show_shortcut_hints`), dead helpers
  hidden behind `allow(dead_code)`, and unused deps (`urlencoding`,
  `fastframe-audio`; `image`/`tungstenite`/`url`/`percent-encoding`
  from melbad).
- [x] **flake.nix verified** — `nix build .#melba` compiles, runs the
  full test suite in the sandbox, and installs desktop/icon files;
  `nix flake check` passes. Repaired during verification: stale vendor
  hash, a projectm-sys postPatch for a removed dep, missing dbus in
  buildInputs, a wrong desktop path, and a Darwin block that still
  named Spotifast and pointed at absent packaging/macos files.
- [~] **Linux-only runtime verification** — all runtime gates ran on
  one Linux host; no macOS/Windows runtime claims. Cross-compile:
  `cargo check --target x86_64-pc-windows-gnu --all-targets` passes
  (`dbus`/`zbus` deps are Linux-gated, `daemon_worker` behind
  `#[cfg(target_os = "linux")]`, CI checks every push). macOS is
  blocked by the absent Darwin SDK/toolchain, not by the code.

- [ ] **Optional ALAC wrapper path** — the user-supplied decrypt route is not
  part of this source distribution; provisioning compatibility remains
  untested here. Runtime-component boundaries are in
  [NOTICE.md](NOTICE.md#runtime-components-and-research-materials).

Done recently (kept for context): the dead-feature excision in two
passes (the inherited librespot engine plus the visualizer/EQ/Winamp
skin/mini-player/MilkDrop/rootlist/contributor/engine-era settings
chains — 119 files total across both commits, ~61,000 lines removed,
dead dependencies dropped from the manifest), the i18n catalog prune
(dead msgids removed from the template and all 16 catalogs, with the
template↔catalog parity test re-verified), the `aml`→`melba`/`melbad`
rename end to end (daemon, CDM crate + FFI, MPRIS bus, config/cache/state
paths, gettext catalogs in 16 locales, fixtures, scripts, packaging, CI
models), browser sign-in loop completed
in Melba (waiting card, cancel, failure paths, revoked-token recovery),
Apple Music share-link parsing and copy-link on `music.apple.com`,
bidi layout fix on ligature-shaping fonts, ALAC end-to-end verification
through an external decrypt service, queue drag-reorder via `MelbaQueueReorder`,
and `melba:artists`/`melba:artUrls` queue props.

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
