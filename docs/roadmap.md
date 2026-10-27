# Roadmap and project status

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
  selected title. Follow the [runtime verification policy](../crates/melba/AGENTS.md#runtime-verification).
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
  [the desktop lifecycle plan](../specs/desktop-lifecycle-update/plan.md), not a
  claim that the installed client has passed acceptance.
- [x] **Paired signed self-update** — v0.3.6 ships the daemon inside the GUI
  archive; the vendored fastframe helper replaces both and restores both when
  the new GUI fails. Verified with a broken v0.3.7 test release: receipt
  "Update failed; restored the previous app", both binaries back at v0.3.6 (test build).
- [x] **GUI update button acceptance** — portable v0.3.2 → v0.3.3 was driven
  through Settings → About and the update modal on 2026-10-26: signed download,
  install, helper restart and `melba 0.3.3` startup succeeded. The marker
  (`melba-portable.txt`) is required. The default installer now installs that
  portable layout, and `packaging/package-portable.sh` checks the marker.
- [x] **v0.3.6 signed-update acceptance** — a portable v0.3.5 installation
  checked, downloaded and installed the signed v0.3.6 release on 2026-10-27;
  both binaries report 0.3.6 and the receipt read “Updated to 0.3.6”. The
  v0.3.6 archive carries the complete Spotifast/third-party notice set.
- [x] **Paired update rollback** — the portable installer and the v0.3.6 GUI
  helper both swap `melba` and `melbad` together and restore both on failure.
  GUI rollback was exercised with a deliberately broken test release; the
  installer path has a package-level rollback test.
- [x] **arm64 build path** — aarch64 release archives are built on the
  `ubuntu-24.04-arm` runner and published with the same signed manifest as
  x86_64 in v0.3.4, v0.3.5 and v0.3.6. The x86_64-only native auth bridge is disabled on arm64
  (browser sign-in); arm64 binaries are build-verified, not hardware-run.
- [x] **Choose the output from the app, over PipeWire** — `audio_device` in
  `~/.config/melba/config.json` selects `auto`, `default` or `hw:X,Y`;
  `auto` prefers a USB/analog hw device over HDMI, the Settings page lists
  outputs and saves the choice, and `--require-bitperfect` refuses `default`.
  Still open: following the system default automatically and showing the
  opened rate/verdict beside the picker.
- [~] **First-run audio setup** — `melbad setup` and the Settings picker work;
  a guided first-launch wizard remains.

- [x] **Busy output takeover** — a busy `hw:` output remains in Settings as
  “in use”; selecting it requests the session audio server's device
  reservation release through D-Bus and then opens the direct device. The
  takeover is explicit, stops other audio on that card, and keeps the current
  output unchanged if the release is refused.
- [x] **HDMI/AVR bit-perfect verification** — a Yamaha RX-V6A on `hw:0,7`
  reported a valid ELD (LPCM 8ch, 16/20/24 bits, 32–192 kHz). A 96 kHz/24-bit
  FLAC, a 192 kHz/24-bit WAV and Apple Music ALAC 24/192 all produced
  `BITPERFECT` receipts in the ALSA digital path; see
  [the evidence](../evidence/hdmi-avr-bitperfect.md). Analog output, volume,
  room correction and other AVR DSP remain outside this claim.
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
  [NOTICE.md](../NOTICE.md#runtime-components-and-research-materials).

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
