# Signed releases and updates

[README](../README.md) · [Installation](installation.md) · [Roadmap](roadmap.md)

The GUI's **Check for updates** action (Settings → About and the app menu) checks the latest GitHub release. A source tag being newer does not make it installable: installation needs suitable signed binary assets and a supported **portable** layout.

## Supported layout and update behaviour

The GUI updater requires `melba-portable.txt` beside the executable. The default `packaging/install.sh` installation puts this marker with `melba` and `melbad` in `~/.local/opt/melba`; an ordinary `cargo install` under `~/.cargo` does not, and cannot update through the GUI. Distribution packages and source installations should use their own upgrade procedure.

Current GUI release archives include the companion `melbad`, and the update helper replaces the pair together. If the new GUI fails to start, the helper restores the previous pair. A missing package, bad signature or failed startup is not reported as a successful update. The standalone installer also stages the pair and checks for matching versions.

## Signature boundary

The updater accepts release downloads only with a valid `checksums.txt.sig`: an Ed25519 signature over `checksums.txt` verified against publisher public keys shipped with the application. A GitHub checksum file alone does **not** provide independent publisher authentication. The default verification key is in `crates/melba/assets/update-public-key.hex` and cannot safely be removed without changing the trust model.

The published key is a verification trust anchor; it does not itself prove who controls the corresponding private key. Before releasing, the publisher must confirm provenance/control, keep the private signing key outside the repository with restricted permissions, and test a real signed archive against the key shipped in the app.

With `MELBA_SIGNING_KEY` set to the publisher's private key file, the signing operation is:

```sh
openssl pkeyutl -sign -rawin -inkey "$MELBA_SIGNING_KEY" -in checksums.txt -out checksums.txt.sig
```

Do not commit the private key or treat a signing receipt as a full legal or security audit.

## Preparing and publishing a new signed Release

The fully CI-validated v0.3.13 release preparation is at
[PR #33](https://github.com/nekomario28/melba/pull/33). The `portable-baseline`
workflow builds x86_64 and aarch64 on Ubuntu 22.04 (glibc 2.35), then tests
the **actual unsigned candidates** in eight supported Linux containers.
These are **not** public trusted Release assets until signed.

After the source version update is merged into `main`, the repository maintainer
must register the **existing** Ed25519 private publisher key in GitHub
**Settings → Secrets and variables → Actions**, as the repository secret
`MELBA_SIGNING_KEY_PEM` (or a secret with that name in the protected
`release` environment). Use the complete PEM private-key content, never a
newly generated key: already-installed clients trust the pinned public key.

To register the PEM from a private local file without printing it to a shell
history or placing it in the repository, the maintainer can use:

```sh
gh secret set MELBA_SIGNING_KEY_PEM < /secure/path/to/existing-publisher-key.pem
```

Once the secret exists, go to **Actions → Publish signed portable Release →
Run workflow**, select **main**, enter `0.3.13`, and check the explicit
publication confirmation. Alternatively:

```sh
gh workflow run release-publish.yml --ref main \
  -f version=0.3.13 -F confirm_publish=true
```

The workflow rejects existing tags, mismatched source versions, a missing or
incorrect private key, missing or malformed release assets, failed ABI tests,
or any failed distro smoke. Only after all gates succeed will it create a public
Release with both binaries for x86_64 and aarch64, the four archive SHA-256
entries, the existing publisher's Ed25519 signature, and public key files.
After publishing it downloads those assets and verifies them again. The
`release` environment can be configured with required reviewers in
GitHub's environment protection settings for an additional approval gate.

The published-release compatibility table was refreshed only after both of
its conditions held: v0.3.13 is publicly visible, and the workflow's
`install-release.sh` jobs installed that signed release in real Debian, Ubuntu,
Fedora and Arch containers. `crates/melba/nix/paired-release.nix` now pins the
**v0.3.22** `melba` archive hashes; the fetch is a plain HTTPS fetch of the
signed release asset, so a Nix build still depends on GitHub availability.

## Published releases and tested paths

Signed **v0.3.22** release assets are the current ones for `x86_64` and `aarch64`, from the same source tag, with a detached signed checksum manifest. v0.3.13 through v0.3.22 are built against a **glibc 2.35** baseline (v0.3.12 and older required 2.43), so Debian 12/13, Ubuntu 22.04/24.04/26.04, Fedora 43/44 and current Arch/CachyOS can run the published binaries; see [Installation](installation.md#distro-compatibility-of-the-published-v0322-release) for what was actually executed on each. The arm64 binary is build-verified on a native arm64 GitHub runner but not run-verified on arm64 hardware. The native SDK presentation bridge is x86_64-only; arm64 uses browser sign-in. The optional decrypt wrapper/rootfs remains architecture-specific and user-supplied.

The v0.3.5 and older portable archives contain `NOTICE.md` and the project licences but predate the complete notice-packaging fix: they do not contain `SPOTIFAST-LICENSE` or the generated third-party bundles. v0.3.6 archives carry them. Do not treat an older archive as a complete binary notice set.

v0.3.6 added the complete binary notice set; v0.3.7 added automatic takeover of a busy direct output; v0.3.8 made a manual update check open the update dialog and install itself when idle; v0.3.9 sized the ALSA buffer by time (fixing audible gaps at high sample rates); v0.3.11 made the daemon restart actually run; v0.3.12 hands the started daemon this app's own directories, keeps test processes out of the user state directory, records who writes the saved queue, and regenerates the gettext template (`SPOTIFAST-LICENSE`, both generated dependency bundles and the receipt) and updates rustls, quinn-proto and the bundled CA data. v0.3.13 lowers the portable release baseline to glibc 2.35, verifies Debian, Ubuntu, Fedora and Nix install paths, installs missing runtime libraries through the native package manager, and can provision late ALAC staging tools from `melbad wrapper setup`; the rustls-webpki upgrade to 0.103.16 clears the last advisory that had no fixed stable release. v0.3.14 reads a song's liked state from the account's own rating, so a song loved without being added to the library shows a filled heart, Liked Songs opens the playlist Apple keeps for those songs, and the translation catalogs stop naming a retired repository. v0.3.15 makes that page actually open the playlist: v0.3.14 guarded the lookup with a counter the page load itself moved, so the answer was discarded. v0.3.16 gives a verified shared output its own `PASSTHROUGH` verdict instead of qualifying `BITPERFECT`, stops `melbad play --device default` from playing zero frames, rebuilds an out-of-date login shim instead of asking for `wrapper setup`, and implements the seven commands the daemon used to drop. v0.3.17 stops a queue row being labelled with the slot id the player held (`stream:<id>`) instead of the song, carries the per-slot cover the worker had been parsing and dropping, and stops Home offering a synthetic Liked Songs tile beside the account's own Favorites playlist. v0.3.18 finishes that work: a row that has a name but no cover or length is no longer taken as resolved, a cache entry that describes nothing no longer answers for a song, and a playlist the service never drew artwork for gets a cover derived from its name. v0.3.19 stops reporting `PASSTHROUGH` for a device the session does not carry (`--device null` was claiming another process's stream), drops a retired key from the trusted update keys, checks the token file's permissions instead of assuming them and accepts a token from an external secret command, rotates `wrapper.log` instead of deleting it on restart, reads `config.json` as a struct, and gives `melbad` structured diagnostics. v0.3.20 makes the sign-in shim work on a loader that runs a preload's initialiser before it has set up its arguments — the password never reached the wrapper there — and rebuilds an out-of-date shim instead of asking for `melbad wrapper setup`. v0.3.21 draws a playlist's group icon from its first tracks instead of its name, and asks which library playlist is the account's Favorites when the library loads, so Home stops offering a synthetic Liked Songs tile beside it. v0.3.22 stops a queue that cannot play from walking every entry forever — 62 million advances had left 14 GB of a diagnostic file behind — and removes that recorder. v0.3.23 gives a playlist's cover one owner, so the page header, the tiles, the cards, the grid and search agree on the group icon instead of the header staying empty.

v0.3.7 keeps a busy `hw:` output in the Settings picker as “in use”. Selecting it asks the session audio server, through `org.freedesktop.ReserveDevice1.RequestRelease`, to release the card, then Melba opens the direct device. This stops other system audio on that card; it is the explicit choice to give Melba exclusive control.

v0.3.8 also removes the small-notification-only manual update path: a manual check opens the update dialog and starts the signed download. Once verified, it restarts in place when Melba is not playing. If music is playing, it stays open with the restart choice instead of interrupting playback.

The following cases were recorded on **2026-10-26**, **2026-10-27** and **2026-10-28**:

- The portable GUI v0.3.2 → v0.3.3 update was driven end to end through the GUI and restarted into v0.3.3.
- A portable v0.3.5 installation checked, downloaded and installed signed v0.3.6 through the GUI. The receipt read “Updated to 0.3.6”, both binaries reported 0.3.6, and the downloaded archive matched the signed checksum manifest.
- The v0.3.5 paired-update path moved a portable v0.3.5 installation to a test build labelled v0.3.6 with both binaries; that test release was removed afterwards.
- A deliberately broken v0.3.7 **test** release caused both executables to be restored to v0.3.6; the receipt read “Update failed; restored the previous app”.

The GUI update replaces the executables. The v0.3.6 archive carries the complete notice set, but an existing v0.3.5 installation that predates it should be reinstalled from the current source or a fresh portable archive if the local `share/licenses` tree is required.

These are path-specific tests, not a guarantee for every distro, signing setup or device. The earlier Cargo-root install is not GUI-updatable.

## Key rotation

`crates/melba/src/updates.rs` retains additional verification keys under `additional_publisher_keys`. v0.3.2 adopted publisher key `1e47…` while temporarily retaining `e44c…` for old signatures. v0.3.1 and older do not recognize the newer signing key and require a manual/package update before relying on newly signed releases. The old key's private half is unavailable, so an old-key-signed transition release cannot be generated.

This transition is a compatibility constraint, not evidence that all older versions can update automatically. See the [current releases](https://github.com/nekomario28/melba/releases), [Roadmap](roadmap.md) and [PUBLICATION.md](../PUBLICATION.md).
