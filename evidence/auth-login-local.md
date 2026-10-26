# Browser sign-in: local verification

Base: `9252883`, plus the browser-login changes accompanying this record.
No Apple account was used and no real user token was captured.

## Implemented

- `aml auth login [--browser <executable>]`, also reachable from Settings in
  both shipped languages. Browser work runs outside the UI paint path.
- A newly created 0700 profile, random loopback CDP port, and the actual
  `https://music.apple.com/` page. No developer token is embedded in a local
  page; the unauthenticated localhost token-POST prototype was removed.
- CDP discovery validates the exact HTTPS origin and loopback socket endpoint.
  The read expression checks the current origin and MusicKit authorization
  again before returning the token. Passwords, MFA and cookies are not read.
- Token files are created 0600 before writing and renamed atomically;
  symlink destinations are not followed. Status output omits token prefixes.
- Closing the dedicated window or timing out preserves an existing token.
  Normal completion/error removes the temporary profile. A parent-death
  signal terminates the browser if aml dies, but an abrupt kill can leave
  a private `~/.cache/aml/login-*` directory. Do not share these directories.

## Observed

- `cargo test --locked --offline`: 30 tests passed (19 library, 3 auth CLI,
  8 bit-exact playback tests).
- `cargo build --locked --offline --bin aml --bin aml-ui`: PASS, development
  profile, bounded to 2 GiB / 150% CPU.
- Real local HTTP/WebSocket fixtures exercise discovery, an initial null
  token, an authorized dummy token, private storage and browser cleanup.
  No fixture connects to Apple.
- Three regression controls were run in an isolated source copy with old
  behavior injected: loose domain matching, symlink-following storage, and
  token-prefix status output. Each named test failed with exit 101; the
  real worktree's corrected tests passed.
- `ui-auth-settings-ja.png` and `ui-auth-settings-en.png`: actual wgpu
  development-build renders, inspected in both languages. These prove
  layout/presence, not a successful account sign-in or GUI click delivery.

## Not Proven / Next Action

- Apple login/MFA, the live website's MusicKit state exposure, and acceptance
  of the resulting user token by account/lyrics endpoints: **NOT_RUN**.
  The user must perform sign-in; API access is not inferred from token storage.
- Release build: deferred after the resource lease gate reported another
  heavy compilation. The local trial may use `target/debug/aml{,-ui}` instead;
  do not label development-build evidence as release evidence.
- Full `ci/check.sh` on a new release binary: **NOT_RUN**.
- For this host's trial, the existing `~/.cargo/bin/aml` and `aml-ui`
  symlinks now point to the verified development binaries. Their previous
  targets were this repository's `target/release/aml` and `aml-ui`.
  Already running app windows were not restarted.
- An existing license-checker issue was observed: `ci/licenses.sh` printed
  `FORBIDDEN LICENSES: hexf-parse-0.2.1: CC0-1.0` yet exited 0 because its
  final `echo` masks the Python failure. That dependency and script behavior
  predate this change; the only lockfile change here adds the already-resolved
  `url` package as a direct dependency. Do not interpret this check as PASS or
  relax the license rule just to obtain green. This issue is recorded, not
  repaired by the sign-in change.
- Next: with permission for external access, open the dedicated browser and
  have the user sign in. Verify token presence/permissions without displaying
  its value, then test a bounded read-only account endpoint. Do not add library
  items, authorize purchases, or publish session-bearing artifacts.

## Subsequent UX Decision: Use The Normal Default Browser

The user requested the normal default browser so saved passwords remain usable.
`xdg-settings get default-web-browser` reports `vivaldi-stable.desktop`.
The isolated trial ended when its window was closed; no token was stored.
With the user's permission, `xdg-open https://music.apple.com/` was invoked
successfully, without enabling debugging or inspecting the normal profile.

Opening the website is **not** authentication delivery to aml. The current
`auth login` implementation and Settings button still use the isolated route;
they have not yet been converted to a normal-profile handoff. Do not restart
that route as though it met the updated requirement.

Next decision: obtain approval for a user-initiated browser handoff, for example
a narrowly scoped extension that sends the authorized MusicKit token to aml on
an explicit "Connect aml" action. Do not enable debugging on the user's normal
profile, read its password database/cookies, or call the normal-browser flow
complete before a real token reaches the application.

## Resolution: `aml auth sync` — Normal-Browser Token Import

The extension route was not needed. MusicKit stores its authorized
`media-user-token` in Chromium-family `Local Storage/leveldb`, so
`aml auth sync` (and the Settings "sync from browser" button) imports it from
the normal browser profile with a read-only scan:

- Profiles searched: `AML_AUTH_PROFILE` if set, else Vivaldi, Chromium,
  Chrome, Brave, Edge `Default` dirs — no writes to the profile, and password
  databases/cookies are never opened.
- Extraction requires a `user-token`/`userToken` record key near a JWT-shaped
  value (ASCII or UTF-16LE localStorage encodings); the newest non-expired
  `exp` wins; expired tokens are rejected with a sign-in message.
- The token is stored through the existing atomic mode-0600 path and never
  printed.

Verified on this host: the command honestly reports
"no MusicKit sign-in found ... sign in on music.apple.com" (14 files scanned)
because the user had not signed in there yet, and unit fixtures cover ASCII +
UTF-16LE extraction, newest-exp selection, and the empty-profile message.

**Still unverified live:** a real token reaching aml from a signed-in browser
session — requires the user to sign in on music.apple.com in the normal
browser once, then run `aml auth sync`. The isolated `aml auth login` route
remains available as fallback.


## Removed (2026-10-15)

Per user direction the browser-mediated auth paths were removed as stale:
`aml auth login` (isolated browser + CDP capture), `aml auth sync` (LevelDB
localStorage import), the Settings buttons, and the `aml bridge` CDP module.
Remaining: `aml auth set|status|clear|how` (manual token paste, 0600).
Subscription playback now targets the licensed Widevine CDM host
(crates/aml-cdm), which made the browser-mediated routes obsolete.


## Final auth UX (2026-10-15): browser handoff + aml:// return

`aml auth open` opens music.apple.com in the normal browser; after sign-in
the user clicks a one-time-installed bookmarklet which reads
`MusicKit.getInstance().musicUserToken` and navigates to
`aml://auth/<token>`. The `aml` scheme is registered to aml-ui; the same
deeplink parser boundary validates JWT shape, the UI checks expiry, and
stores via the existing atomic 0600 path. Verified end-to-end:
`aml-ui 'aml://auth/<fixture-jwt>'` -> token file written 0600 in a
scratch XDG_CONFIG_HOME. A live Apple sign-in round-trip is still the
user-side step, but the mechanism is exercised and the security boundary
is unchanged (no cookies/passwords touched; the page JS is user-clicked).


## In-app sign-in window (2026-10-15, replaces bookmark-as-primary)

`aml auth open` now hosts music.apple.com inside a WebKitGTK webview
(wry 0.53 + tao 0.34). An injected page-side poller pushes
`MusicKit.getInstance().musicUserToken` to Rust via `window.ipc`, where
`auth_webview::take_token` checks JWT shape + expiry and `auth::set`
stores it 0600. WebContext persists at ~/.local/share/aml/auth-webview
so the Apple session survives restarts. If webview construction fails
(e.g. no webkit2gtk) the command falls back to the external browser +
aml:// bookmarklet path. Verified: `aml auth open` launches the window
cleanly on this host (webkit2gtk-4.1 2.54.1); earlier failure mode
(window-handle-kind unsupported) fixed via build_gtk(default_vbox()).
A real sign-in round trip still needs the user's account, but the
capture path is exercised and the security boundary is unchanged: the
token never touches logs, the clipboard, or argv.


## Final shape (2026-10-16): browser sign-in + localStorage watch

The in-app WebKitGTK window reached a hard boundary: sign-in itself
completed (popup flow worked) but MusicKit reported
authorizationStatus=3 (restricted) and never issued a user token —
Apple restricts token issuance in embedded WebKit environments. The
webview remains available via `aml auth open --webview` for diagnosis;
STATUS transitions print over IPC on stderr.

Default flow is now: `aml auth open` → xdg-open music.apple.com in the
normal browser → `auth::watch_browser` polls the Chromium profile's
`Local Storage/leveldb` every 2s for MusicKit user-token records
(read-only; only records matching user-token keys near JWTs; newest exp
wins; expired ignored) → atomic 0600 store. Verified on the host: a
scan over vivaldi/Brave profiles currently finds no token because no
normal-browser sign-in exists yet — the watch loop is the correct UX
for that gap. Sync scanner + fixtures restored from history with unit
tests (ASCII + UTF-16LE extraction, newest-exp selection, empty-profile
message).


## Resolution (2026-10-16): the sign-in had worked all along

authorizationStatus=3 (restricted) turned out not to be the real
boundary: Apple refuses to mint the MusicKit JS token for the embedded
context, but the auth redirect chain still sets the HttpOnly
`media-user-token` cookie on .music.apple.com — an opaque `0.<base64>`
blob (246 bytes), not a JWT. Verified end-to-end: the cookie value sent
as the `media-user-token` header on `/v1/me/storefront` returns 200 with
the user's real storefront. `aml auth open --webview` captured the
already-persisted cookie instantly and `auth status` reports stored.

Current default: `aml auth open` = the in-app window (cookie channel +
JS/IPC channel both armed). `--browser` = normal-browser localStorage
watch. `auth status` shows `exp unknown` for opaque tokens — correct,
nothing to decode.
