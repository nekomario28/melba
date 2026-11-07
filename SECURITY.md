# Security policy

Melba plays audio from Apple Music on Linux through a user-supplied decrypt
service. This page says how to report a problem, what a reported problem is
judged against, and what to do when a credential or a release key is
compromised.

The threat model — what is stored, what an attacker would need, and what is
deliberately out of scope — is in [docs/security.md](docs/security.md).

## Reporting a vulnerability

Use GitHub's private reporting: **Security** tab → *Report a vulnerability* on
<https://github.com/nekomario28/melba>. If that is unavailable, open an issue
that says a private report is needed and nothing else; do not put details,
tokens or logs with credentials in a public issue.

Please include the version (`melba --version`), distribution, and the smallest
reproduction you have. `melbad doctor` output is useful and contains no
credentials; `wrapper.log` can, so read it before attaching.

This is a single-maintainer project. Expect an acknowledgement within a week
and an honest answer about whether and when a fix will land; there is no
bug-bounty program and no response-time guarantee beyond that.

## What counts

In scope:

- the token file, its permissions, and the credential handoff through the memfd
  (a password that reaches `/proc/<pid>/cmdline` is a bug, not a limitation);
- the sign-in webview and browser profile import (a page that can drive token
  changes, or an import that accepts a record from another origin);
- the release pipeline: signing, checksum publication, and the installer's
  verification of both;
- the wrapper connection policy (a non-loopback URL that is accepted without
  the documented opt-in);
- anything that makes Melba write a credential somewhere it says it does not.

Out of scope, by design:

- the decrypt service, the Android rootfs and the Widevine CDM are supplied by
  you, not by this project — report those to their authors;
- DRM circumvention, or requests to bundle Apple binaries;
- an attacker who already runs code as your user, or root;
- a compromised browser profile of your own.

## If a credential is exposed

**MusicKit user token** (the value behind `melbad auth status`):

1. `melbad auth clear` removes the stored copy and the native-sign-in marker.
2. Sign out of Apple Music on the device or in the browser the token came from.
3. Sign in again. Melba rewrites the token file with mode `0600` in a `0700`
   directory; `melbad auth status` reports `storage:` if it is not.
4. If the token came from `MELBA_MUSIC_USER_TOKEN_CMD`, rotate it in that store
   instead — Melba never wrote it.

**Apple ID password**: Melba never stores it. If it was exposed, change it at
Apple; nothing here needs updating.

**A wrapper or rootfs you staged**: stop the service (`melbad wrapper stop`),
remove `$XDG_DATA_HOME/melba/wrapper/`, and stage a fresh copy. Its ports are
unauthenticated, so anyone who could reach them could already use it.

## If the release signing key is exposed

Release assets are signed with an Ed25519 key; each client pins the public key
it was built with (`crates/melba/assets/update-public-key.hex`, compiled into
`updates.rs`) and refuses updates signed by another.

`updates.rs` also carries `additional_publisher_keys`, a list of keys that are
still trusted besides the pinned one. It exists for transitions: a client can
be told to trust the next key by a release signed with the key it already
trusts. **Anything on that list can sign an update this build accepts**, so it
is a temporary measure, and removing an entry is part of finishing a rotation.

Rotation is therefore ordered:

1. Generate a new key. Keep the old one available: it is still needed.
2. Publish a release **signed with the old key** whose sources add the new
   public key — either as `publisher_key` with the old one added to
   `additional_publisher_keys`, or the other way round while the new key is
   proven. Clients on that release trust both. Announce it and say the old key
   is retiring.
3. After that release is out, sign subsequent releases with the new key, and
   replace the `MELBA_SIGNING_KEY_PEM` value in the `release` environment
   secret (replace, do not add a second secret).
4. Drop the retired key from `additional_publisher_keys`. Anyone still on a
   release from before step 2 will refuse the new signature; that is the
   pinned-key behaviour working as intended, and is why step 2 exists.

If the key is exposed and no rotation release has gone out, the same steps apply
with urgency, and the release notes must say the old key is compromised rather
than retired.

## Incident notes

- Checksums, the detached signature and the receipt are published per release;
  a release whose assets do not verify should be treated as not having
  happened, and reported.
- Nothing in Melba reports anything automatically: no telemetry, no crash
  upload. Logs stay on the machine (`melba.log`, `panic.log`,
  `wrapper.log`/`wrapper.log.1`), which is why a report has to carry them.
