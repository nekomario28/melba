# Web player's "Open in Music": native handoff

Consumer: aml default-browser sign-in / desktop handoff work.
Repository base: `ef4717e`. Observation time: 2026-10-15T16:56:02Z.

The named album/playlist PNGs now contain [synthetic documentation illustrations](../evidence/README.md), not the original live captures. They do not substantiate the historical service or GUI observations below.

## Question And Boundary

Can the existing web-player button connect the user's normal browser to aml,
instead of requiring a browser extension or an isolated debugging profile?

Only public, unauthenticated Apple source was fetched. No user browser session,
password database, cookies, or account data was inspected. Downloaded JavaScript
was read as text, not executed. Apple's implementation is REFERENCE_ONLY;
do not copy its source into aml.

## Source Identity

- Page: <https://music.apple.com/us/browse>, 2,369,202 bytes on the first fetch,
  SHA-256 `51b4b27f6d78b2a2873db3def4663878a196966920410f93e9ed709149f70d98`.
- Referenced application bundle:
  <https://music.apple.com/assets/index~c10ba4a68d.js>, 3,308,852 bytes,
  SHA-256 `c9786b220773aca5bede1fe0f9e19991118ebc89f3e784656b0d7adee6553bf9`.
- The page contains a `native-cta-button` labelled "Open in Music".
  It is a JavaScript button, not a plain anchor with a final external URL.

## Static Trace

Names below identify this exact minified bundle, not a stable Apple API.

1. CTA instance `fAe` constructs its open action using the current page URL
   and the result of routing that URL; it calls `aa`.
2. `aa` calls `xEe` to produce the handoff URL, then `nr.launchClient`.
3. `xEe` uses the `music.apple.com` origin and `app=music` by default.
   Library/browse/radio navigation may become `/deeplink` with a `p` value.
   The inspected code adds no MusicKit user token or authorization code.
4. `launchClient` calls `jG`, then `YG` / `KG` to select an external scheme.
   The `WG` table chooses `music` for eligible Apple platforms, `apple-music`
   for Android, and ends with an unconditional `itms` fallback.
   Thus the inspected Linux branch derives an `itms://music.apple.com/...`
   URL, retaining the relevant path/query.
5. Outside special Safari / legacy Windows cases, `jG` assigns the resulting
   URL to the top-level location. The OS/browser external-protocol handler
   is the seam aml could implement.

This is static source evidence, not an observed real-button click or a
successful native application launch. It does not prove native login sharing.

## Existing Host Owner

Read-only queries returned:

- Default browser: `vivaldi-stable.desktop`.
- `x-scheme-handler/itms`: `apple-music-semantic.desktop`.
- `x-scheme-handler/music` and `musics`: no default reported by `xdg-mime`.

The current itms desktop entry invokes `~/.local/bin/apple-music %U`.
That existing shell script selects Sidra, a browser, or Android/Waydroid;
it is not aml. It was read, not executed. Do not overwrite or invoke that
route without accounting for its existing ownership and side effects.

aml's packaged desktop entry currently has neither `%U` nor these scheme
MIME types. Its UI treats the first argument as a library source, so merely
changing the OS association would not implement correct deep-link handling.

## Decision / Next Safe Action

- ADOPT the existing OS external-protocol seam for app launching; no extension
  is needed solely to open aml on a song/album/navigation link.
- Keep authentication separate: the inspected button does not construct an
  aml authorization callback or transfer the browser's cookies/user token.
  Do not claim that registering itms signs aml in.
- Before changing the existing host association, confirm that the user wants
  to replace the old semantic launcher with aml.
- Implement and test a single validated Apple Music URL parser / UI dispatch
  boundary first, including `/deeplink` navigation and hostile/wrong-origin
  inputs. Do not forward untrusted links to a shell or advertise full-track
  subscription playback when the implementation only offers previews.
- Then register the intended schemes and verify one actual browser click.
  Account-token delivery remains a separate, unimplemented normal-profile flow.

## Implementation Landed (same-day follow-up, user approved)

- `src/deeplink.rs` — single trust-boundary parser. Accepts `itms|music|
  musics|apple-music|https|http` schemes on `music.apple.com` only (no
  credentials, no non-default port, bounded ASCII-safe ids). Returns an
  allowlisted `Link`: song/album/artist/playlist `{id, store}` plus top-level
  navigation. The `/cc/` storefront segment is preserved through the link —
  catalog ids are storefront-scoped. Album links' `?i=<song-id>` (the web
  player's per-track handoff) resolve to `Link::Song`. `/deeplink?app=music
  &p=` maps browse/radio/library/listenNow/home; `app != music` is rejected.
- `aml-ui` argv handles the desktop entry's `%U` — every URL-looking arg
  (scheme-prefix match) goes to `open_link`; rejected links surface the
  parse error in the UI rather than being retried as a library path. The
  first non-URL arg stays the library source.
- Single-instance navigation: a running aml-ui owns
  `~/.local/state/aml/ui-links.sock`; later `aml-ui <url>` processes (each
  desktop click spawns one) forward URLs over it and exit — same-window
  navigation instead of stacked processes. Payload is bounded (8 KiB),
  newline-separated, and re-parsed at `open_link` — argv and socket reach
  the identical boundary.
- `musickit` gained `storefront()` (`AML_STORE` override, default `jp`) and
  `*_in(store,…)` variants (lookup/lookup_song/artist/collection_of/
  preview_url/download_preview/lyrics). `preview:` URIs accept
  `preview:<cc>:<id>`; bare ids keep the app storefront, foreign-store
  preview cache files are `cc-<id>.m4a` so same-id different-store songs
  cannot alias.
- Verified live: `xdg-open itms://music.apple.com/jp/album/…/1748185853`
  spawned `aml-ui <library-dir> <url>` (library path generalized) via the new association and loaded
  the album page; a second `xdg-open` forwarded into the same window with
  no new process. `itms://…/us/album/…/1736268193` rendered against the US
  catalog (`evidence/ui-deeplink-album-en.png`),
  `itms://…/jp/playlist/pl.…` rendered 30 tracks
  (`evidence/ui-deeplink-playlist-ja.png`), `itms://evil.example.com/…`
  shows the rejection text (`evidence/ui-deeplink-rejected-ja.png`), and
  `aml ctl open 'preview:us:1736268194'` played Fortnight's 30 s AAC
  BITPERFECT with us-catalog metadata.
- Association switched per user approval: `x-scheme-handler/{itms,music,
  musics,apple-music}` now default to `aml-ui.desktop`
  (`Exec=… aml-ui <lib> %U`). The old `apple-music-semantic.desktop` and
  `~/.local/bin/apple-music` are left in place, merely no longer default.
- Still NOT: this handoff carries no login state — `aml auth login` remains
  the credential path; subscription playback stays in the browser bridge.
