# Roadmap and project status

### Release readiness priorities

Prioritize a reliable native Melba release over recreating the official UI.
The next release should make audio preferences effective, playback dependable,
and the actual playback path understandable.

Current release: **v0.3.12** (signed x86_64 and aarch64). The entries below
name the release each acceptance was measured on; an older version in an entry
is a dated record, not the current one.

- [~] **Audio quality settings** — `melbad config audio-quality` and a Settings
  control both write `auto`, `aac` or `lossless` to `config.json`; `aac` is
  honoured by never asking the decrypt service, and the page writes that one key
  without disturbing the file's others. Still open: a `hi-res` choice, because
  picking a hi-res ALAC variant apart from a lossless one needs per-variant
  attributes from the master playlist that `melba-cdm` does not parse, and there
  is no live master to check a guess against without an account.

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
  DSP is the one part that cannot be bolted on: each of EQ, crossfade and Sound
  Check alters the samples that reach the device, so `BITPERFECT` would be a lie
  unless the processing is opt-in and the verdict says `DSP`. Its own slice has
  to settle where the processing sits (daemon, before ALSA), what the receipt
  reports, and how the offline and preview paths behave.

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

### Shared-output verification

- [x] **PipeWire pass-through is observable, not assumed** — `melbad doctor`
  reports the session graph for this daemon's stream: the chain of nodes to the
  sink, the format the stream negotiated, whether a converter sits in the path,
  and how many clients share the sink (`src/pipewire.rs`, parsed from `pw-dump`,
  with unit tests for each shape). The same reading decides `PASSTHROUGH`, so
  doctor is how that verdict is checked rather than taken on trust.
- [x] **A verified pass-through is called `PASSTHROUGH`**, not `BITPERFECT`:
  the daemon asks the graph at stream open and re-checks it every five seconds,
  so a second client or a converter drops the word back to `SHARED`. Measured
  once live (`play --device default` printed `PASSTHROUGH`); the five-second
  downgrade has unit coverage through the graph parser but was not re-observed
  on a device, because the test harness wedged the session audio server after
  repeated abrupt kills.
- [x] **A `SHARED` stream whose graph verifies carries its own word** —
  `PASSTHROUGH`, separate from `BITPERFECT`, so the exclusive-device claim keeps
  its meaning. The conditions are the measured ones (no converters, this stream
  is the only client, stream rate equal to the source rate, at most 24 bits) and
  they only apply to the device the session actually carries: `hw:`, `plughw:`,
  `null` and paths are opened here directly, so the graph says nothing about
  them. That distinction was learned the hard way — `play --device null`
  reported `PASSTHROUGH` by reading another melbad process's stream.
- [x] **The five-second downgrade was re-observed, both directions** — measured
  2026-11-09 on a 90 s 44.1 kHz source played to `default` by an isolated
  daemon, polling `melbad now`:
  `PASSTHROUGH` alone; a second client (`pw-play`, same rate) started and the
  verdict stayed `PASSTHROUGH` for 3.9 s, then read `SHARED` at +5.2 s — the
  watcher's poll, not an immediate recompute; the second client stopped and the
  next poll (1.3 s later) read `PASSTHROUGH` again, with the position advancing
  through every reading.
  Two earlier attempts were not evidence: with a 3 s fixture the stream ended
  before the poll, and with a 96 kHz source the joining 48 kHz client made the
  sink renegotiate and the stream closed. Same-rate sources are what the
  downgrade question needs.

### Dead GUI-to-worker surface

- [x] **Commands the worker drops — none left** — `daemon_worker` receives
  commands in one match whose `_ => {}` arm silently discards anything unnamed.
  Thirteen variants were sent by the app and never handled: ten are implemented
  (both liked-songs cache commands, `OpenThemesFolder`, the artwork accent,
  `NetworkStatus`, the playlist cache, the library/home account caches, and the
  two playlist-cover commands) and the radio surface is removed rather than
  answered. `ci/test_daemon_commands.py` records an empty exception set, so a
  variant that is sent and dropped fails the gate instead of joining a list.
- [x] **Playlist and account caches** — `LoadPlaylistCache`,
  `StorePlaylistCache`, `LoadAccountCaches`, `StoreLibraryCache` and
  `StoreHomeCache` carry the account in the command and write per-account,
  validated files; the playlist cache is versioned and keyed by account and
  snapshot, and the app checks the snapshot before adopting it. The worker only
  stores and reads: a worker-side snapshot table made the cache unreadable on
  first open, so the judgement lives in `try_adopt_playlist_cache`.
- [x] **Network status** — reachability is derived from what HTTP requests
  actually did (a transport error marks the account unreachable, any response
  marks it reachable), so a captive portal or a 500 is not reported as "offline".
  `Backend::network_status` publishes it and the app receives `NetworkStatus`
  only when the answer changes.
- [x] **Radio pages removed** — Apple publishes no station endpoint, so
  `Command::Radio` was answered by nothing and a station never listed its songs.
  The surface is gone end to end (page, command, event, menu entries, station URI
  helpers, view, fixtures, tests). A seed-based mix built from the existing
  context requests remains possible if it is ever wanted; it would be a new
  feature rather than a repair.
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
