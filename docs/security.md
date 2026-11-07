# Threat model and credential storage

[README](../README.md) · [Authentication](authentication.md) · [Wrapper control](wrapper.md) · [Installation](installation.md)

This page says what Melba stores, what an attacker would need to reach it, and
what is deliberately out of scope. It exists so a decision like "keep the token
in the desktop keyring" can be judged against a written model instead of a
feeling.

## What is stored, and where

| Item | Where | Protection | What it grants |
|---|---|---|---|
| Apple ID password | **nowhere** | handed to the wrapper process through a memfd at sign-in, never written | full account access, so it is never stored |
| MusicKit user token (`music-user-token`) | `$XDG_CONFIG_HOME/melba/music-user-token` | mode `0600`, parent directory `0700`, atomic replace | catalogue and library reads as you: your playlists, your ratings, your library |
| Apple developer token | memory, plus `$XDG_CACHE_HOME/melba/dev-token` | public by design (it identifies the app, not you) | nothing on its own; Apple rate-limits it per app |
| Settings, window state | `config.json`, `settings.json` | `0644` on purpose: no secrets | local preferences only |
| Wrapper binding | `$XDG_DATA_HOME/melba/wrapper/` | file modes set at creation | a cached wrapper session marker, not the password |
| Widevine CDM | wherever your browser installed it | your browser's own packaging | DRM decryption for AAC playback |

The token is a bearer credential: anyone holding it can make the same requests
Melba makes until it expires or is revoked. It is not the account password and
cannot change account settings, but it is not harmless either.

## What is checked at runtime

- The token file's own mode, and its directory's, are read rather than assumed.
  A file at `0644`, or a directory others can enter, is reported on stderr and
  by `melbad auth status`; the token is still used, because the exposure already
  happened and refusing would lock you out without undoing it. Signing in again
  rewrites it privately.
- A token file that is a symlink is reported: the atomic writer replaces links
  rather than following them, so a link means something else owned that path.
- Replacement is atomic: a temporary file with `0600` created before writing,
  then renamed over the target. A crash leaves the previous token, never a
  half-written one.
- An external secret store can supply the token instead of the file:

  ```sh
  export MELBA_MUSIC_USER_TOKEN_CMD='secret-tool lookup service melba'
  ```

  The command's stdout is used as the token and is validated the same way. This
  is the supported route for a keyring, a password manager or anything else,
  without Melba depending on any of them.

## Why there is no keyring dependency

A desktop keyring (Secret Service, `gnome-keyring`, KWallet) stores an item
encrypted at rest, unlocked with the login session. It changes the answer in
three cases: a backup or cloud-synced home directory that leaks `~/.config`,
another local user who can read your home, and a stolen disk without full-disk
encryption. It does **not** stop malware running as you — it asks the same
session for the same item — and it does not stop root.

Against that, a Melba-side keyring would add a Secret Service dependency and a
new way to fail: a session with no keyring, a locked collection, a D-Bus that
is not there. Every one of those turns into "signed out" with no explanation,
on playback paths that must not fail. The token is also revocable in one step
(`melbad auth clear`, or sign out of Apple Music), which is a better lever than
moving where it is kept.

So the file stays, its permissions are checked instead of assumed, and a
keyring is supported through `MELBA_MUSIC_USER_TOKEN_CMD` for anyone who wants
that trade. Full-disk encryption is the honest answer for a stolen disk.

## What is in scope, and what is not

Melba defends against:

- another user on the same machine reading your token or your settings;
- a copied, backed-up or synced home directory exposing the token through a
  file whose mode is wrong (it is reported);
- a web page driving the app: the sign-in webview exposes no page-to-Rust token
  channel, and browser import only accepts current records for
  `https://music.apple.com` (see [Authentication](authentication.md));
- a wrapper on another host: non-loopback wrapper URLs are refused unless
  `wrapper_allow_remote` is set in `config.json` or `MELBA_WRAPPER_ALLOW_REMOTE=1`
  is exported. The wrapper's own ports are unauthenticated, which is why the
  default is loopback-only;
- a replaced audio path: `melbad play --verify` and the GUI verdict report what
  the device actually negotiated, and `PASSTHROUGH`/`BITPERFECT`/`DSP` are
  measured, not assumed (see [Audio](audio.md)).

Melba does **not** defend against:

- root, or malware running as your user;
- screen capture, or a compromised desktop session;
- Apple's own service, the Widevine CDM, or the wrapper binary: the CDM and the
  wrapper are not distributed by this project, and their behaviour is theirs;
- untrusted audio files, other than what Symphonia's decoders do with them.

## Revoking access

- `melbad auth clear` removes the stored token and the native-sign-in failure
  marker. The GUI offers the same from its account settings.
- Signing out of Apple Music in a browser or on another device does not revoke
  this token; it expires on its own schedule, and `melbad auth status` probes
  the service rather than trusting the local expiry.
- Removing `$XDG_CONFIG_HOME/melba/` and `$XDG_DATA_HOME/melba/wrapper/` removes
  every credential-shaped file this project writes.

## Reporting a problem

Open an issue at <https://github.com/nekomario28/melba/issues> without
credentials in it. If a report needs a token or a wrapper log, replace the
values; every one of them is a bearer secret.
