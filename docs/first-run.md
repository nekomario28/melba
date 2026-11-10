# First play on Linux

[README](../README.md) · [日本語](first-run.ja.md) · [Installation](installation.md)

Melba does not bundle Apple's sign-in session, Widevine CDM or optional ALAC decrypt service. A successful install and a working window are **not** proof of full-track playback.

## Start with AAC before enabling lossless

1. Launch **Melba** from the application menu or run `~/.local/opt/melba/melba` (the default portable installation). Select **Sign in with Apple Music**. Full catalog tracks require an active Apple Music subscription.
2. Use the system audio output in Settings for the first test. The shared `default` output works through PipeWire/PulseAudio; direct `hw:` devices are for deliberate exclusive output. If necessary, use `melbad config audio-device default`.
3. Set **Settings → Stream quality → AAC 256** (or `melbad config audio-quality aac`) while diagnosing; this avoids depending on the optional ALAC service.
4. Play a full catalog track **past 30 seconds**. A playing preview, working search, album artwork or a valid session alone does not prove subscription playback.
5. If a full track will not play, run `~/.local/opt/melba/melbad doctor` and inspect its authentication, developer token, Widevine CDM and audio output checks. Do not paste private values into bug reports.

## Common first-run problems

| Symptom | Check |
| --- | --- |
| Only 30 seconds play | Ensure the subscription/session is valid and a usable Widevine CDM is installed by your browser; Melba does not download a CDM. Run `melbad doctor`. |
| Sign-in works in a browser but not Melba | Try the embedded sign-in via `melbad auth open`, then `melbad auth status`. Importing some browser sessions is unsupported; see [Authentication](authentication.md). |
| No sound, device busy or playback stops | Use `default` output first, then `melbad probe` and `melbad doctor`. Direct `hw:` output can be unavailable when another client owns the device. |
| ALAC unavailable | ALAC is optional and needs an independently supplied runtime. See [ALAC setup](installation.md#enable-alac-lossless); staging it does not guarantee playback. |
| `melbad` is not found | Use `~/.local/opt/melba/melbad` for the default portable installation, or add `~/.local/bin` to PATH. Nix and custom installs use different paths. |

## Report a problem

Open a [GitHub issue](https://github.com/nekomario28/melba/issues/new/choose). Include Melba version (`melba --version`), distro and version, desktop session (Wayland/X11), the steps to reproduce, and whether it affects sign-in, full AAC playback, or audio output. Say which `doctor` checks failed **without sharing tokens, passwords, cookies, account identifiers, credential files, signed stream URLs or complete private logs**.

For maintainer release validation, first sign-in, session restore, token expiry/rejection, sign-out, full-track playback, queue transition and resume still require checks with a consenting test account. CI/container tests do not replace this acceptance test.
