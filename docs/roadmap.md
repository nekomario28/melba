# Roadmap and project status

### Release readiness priorities

Prioritize a reliable native Melba release over recreating the official UI.
The next release should make audio preferences effective, playback dependable,
and the actual playback path understandable.

Current release: **v0.3.12** (signed x86_64 and aarch64). The entries below
name the release each acceptance was measured on; an older version in an entry
is a dated record, not the current one.

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
- [x] **Visible and automatic update flow** — a manual update check opens the
  update dialog and starts the signed download; a verified download restarts
  automatically when playback is idle. Active playback is never interrupted
  by the automatic path.
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
  x86_64 (v0.3.4 onward, up to v0.3.12). The x86_64-only native auth bridge is
  disabled on arm64 (browser sign-in).
- [ ] **arm64 hardware run** — blocked, not deferred: no arm64 machine is
  available here, so `melbad`/`melba` on aarch64 are build-verified only. Do
  not describe an arm64 binary as run-verified until someone runs it on that
  hardware and records the receipt.
- [x] **Choose the output from the app, over PipeWire** — `audio_device` in
  `~/.config/melba/config.json` selects `auto`, `default` or `hw:X,Y`;
  `auto` prefers a USB/analog hw device over HDMI, the Settings page lists
  outputs and saves the choice, and `--require-bitperfect` refuses `default`.
  Still open: following the system default automatically and showing the
  opened rate/verdict beside the picker.
- [~] **First-run audio setup** — `melbad setup` and the Settings picker work;
  a guided first-launch wizard remains.
- [ ] **A paused daemon must not hold the output device** — `melbad serve
  --paused` opens the exclusive `hw:` PCM and pauses it, so the card stays
  away from every other client for as long as the desktop is open. Measured
  2026-10-28 on `hw:0,7`: `fuser` showed the paused daemon holding the device.
  Not implemented yet, and the reason is a contract, not effort:
  `tests/transport.rs::desktop_startup_is_paused_and_mpris_actions_reach_the_frontend`
  asserts `status == "Paused"` **and** `rate == 96000` at startup, and that
  rate is published from the opened stream. Not opening the device therefore
  needs `melba:sampleRate` to come from the decoded source instead of the
  granted stream, which is a change to the startup contract and needs its own
  verification (transport test, GUI acceptance, and a check that the device is
  actually free with the app open). Do not ship it without those.

- [x] **Busy output takeover** — a busy `hw:` output remains in Settings as
  “in use”; selecting it requests the session audio server's device
  reservation release through D-Bus and then opens the direct device. The
  takeover is explicit, stops other audio on that card, and keeps the current
  output unchanged if the release is refused.
- [x] **HDMI/AVR playback gaps** — diagnosed as the fixed 16384-frame ALSA
  buffer (85 ms at 192 kHz) against a single-threaded decode-and-write loop,
  not the driver's FIFO warnings (reproduced with stock `aplay`). The buffer is
  now a duration (500 ms) and recovered underruns are counted and published.
  See [the investigation](../evidence/hdmi-audio-dropouts.md).
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
- [x] **Binary release audit** — the gates run on every release input: the
  daemon, GUI and packaging suites, the license/notice bundle check, the
  secret scan, and a download of the published assets checked against
  `checksums.txt` and its Ed25519 signature. Test isolation was the last
  piece (v0.3.12). Still open: a native-package pipeline
  (`packaging/native-packages.yaml`, untested).

Keep requested quality, opened-source format and output-path properties
separate. Do not expose controls for processing the daemon does not implement,
or infer bit-perfect playback from an ALAC label.

### Publication

- [x] **Keep the project's MIT/Apache-2.0 terms.** `NOASSERTION` was rejected:
  removing a license does not reduce DRM or contract exposure, drops the
  patent grant, and makes use and redistribution unclear while dependency
  notices (MIT/MPL/Apache) remain. Risk is managed by scope, not by removing
  the license.
- [ ] **Download/export stays outside Melba.** Saving full tracks is a
  separate product with its own legal review; Melba remains playback-only.

### Deferred presentation

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
- [x] **Release binaries** — signed `x86_64` and `aarch64` archives have
  shipped since v0.3.1, with a detached Ed25519 signature over
  `checksums.txt`, the publisher public key as a release asset and a download
  page on the release. `packaging/package-portable.sh` fails closed when the
  generated notices are missing. Still open: a native-package pipeline
  (`packaging/native-packages.yaml`, untested) if the maintenance is worth it.

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

- [x] **Gettext template and catalog parity** — the shipped template had
  drifted from the sources (37 messages missing, 3 dead, two msgids
  registered as `\u{201c}` escapes and therefore never translatable) and
  `POTFILES` named two files that no longer exist, so the documented
  regeneration recipe could not run. Regenerated in v0.3.12, Japanese
  complete again, and the catalog test now runs in CI — it never had, because
  CI ran `--lib` only. The earlier claim that this parity "was re-verified"
  was wrong: the test did not run and would have failed. A catalog that wraps
  a msgid across lines must not be added; the test's PO library reads the
  continuation as an empty msgid.
- [ ] **Optional ALAC wrapper path** — the user-supplied decrypt route is not
  part of this source distribution; provisioning compatibility remains
  untested here. Runtime-component boundaries are in
  [NOTICE.md](../NOTICE.md#runtime-components-and-research-materials).

Completed work is recorded in the git history, not here: this file lists what
is still open, what is blocked, and what each item is waiting on. A summary of
what already shipped belongs in the release notes.
