# Apple Music sign-in and credential handling

[README](../README.md) · [Wrapper control](wrapper.md) · [Roadmap](roadmap.md)

Full-track subscription playback requires an eligible Apple Music account and a valid `music-user-token`. The browser and native wrapper sign-in paths are distinct; do not treat a working token or fake SDK test as proof that all sign-in methods are supported end to end.

## Normal sign-in

From `melba` choose **Sign in with Apple Music**. The browser flow opens `music.apple.com`, where saved passwords and multifactor authentication can be used. An embedded sign-in webview is also available. The daemon CLI exposes:

```sh
melbad auth status   # saved-token status and live validation; never displays the token
melbad auth open     # embedded music.apple.com window
melbad auth open --browser
melbad auth clear    # remove saved token and native-path failure marker
```

The embedded sign-in webview captures a `media-user-token` cookie; it does not expose page-to-Rust token IPC. Browser import only considers current records for `https://music.apple.com`, not other origins or deleted LevelDB entries. It copies the browser database into bounded memory rather than opening the original for recovery or writes. IndexedDB-only sessions are not imported: unsupported record formats are rejected, not raw-scanned. Use the embedded window or a supported localStorage/Firefox session instead.

Firefox extraction requires the `sqlite3` CLI (`sqlite3` on Debian/Ubuntu, `sqlite` on Fedora/Arch).

## Storage and validation

Before replacing the saved token, automatic capture checks the candidate against Apple's `/v1/me/storefront` API. A parseable JWT expiry is not authentication. Failed live validation preserves the previous credential.

The saved token is **an unencrypted, owner-only file (mode 0600)**, not an OS keyring entry. Embedded browser session data is kept separately at the XDG data directory's `melba/auth-webview`. The embedded profile's permissions are tightened even under a permissive umask; existing cookies are also tightened when reused.

`melbad auth clear` removes the saved token and the native failure marker. It does **not** revoke Apple's server-side authorization, sign out your normal browser, or automatically delete the embedded webview's session. Do not attach any of those data stores to diagnostics.

A manual `melbad auth set` import is explicitly **unvalidated**. Putting a token in an argument can leak through shell history or process listings; use the GUI sign-in flow instead.

## Experimental native wrapper sign-in

When an already-installed compatible decrypt service supplies a native SDK prompt, Melba can relay account credentials, password and verification codes through a private local pipe into that user-supplied runtime. It does not put the password in command-line arguments, a network listener, disk logs, settings or retained UI state.

The SDK provides the prompt text, offered verification methods and labels; Melba relays the selected choice. Selecting SMS/voice verification does not itself prove delivery. The C bridge is pinned to the ABI of the inspected payload, and a changed build fails with `unsupported native authentication ABI`.

This path is **experimental and not part of the supported public configuration**. Prefer browser/embedded sign-in (`melbad auth gui` or `melbad auth open`, as supported by the current build), which avoids passing credentials into a separate runtime. Following failed native enrolment, a marker blocks continued native-path use until a valid sign-in; `auth clear` removes that marker and the saved token.

## Verification boundary

Origin filtering, browser DB recovery and local permission handling have regression coverage with synthetic credentials. An isolated, credential-free probe reached the real SDK sign-in dialogue; credentialed native acceptance and SMS/voice delivery remain **unverified**. An existing token's successful API access or playback is not proof of a fresh native sign-in.

Revalidate live account flows following security-sensitive changes, and never publish Apple IDs, passwords, session tokens, authentication databases or signed media URLs. For unresolved acceptance work, see [Roadmap](roadmap.md).
