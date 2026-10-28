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

## Published releases and tested paths

Signed **v0.3.10** release assets are published for `x86_64` and `aarch64` from the same source tag, with a detached signed checksum manifest. The arm64 binary is build-verified on a native arm64 GitHub runner but not run-verified on arm64 hardware. The native SDK presentation bridge is x86_64-only; arm64 uses browser sign-in. The optional decrypt wrapper/rootfs remains architecture-specific and user-supplied.

The v0.3.5 and older portable archives contain `NOTICE.md` and the project licences but predate the complete notice-packaging fix: they do not contain `SPOTIFAST-LICENSE` or the generated third-party bundles. v0.3.6 archives carry them. Do not treat an older archive as a complete binary notice set.

v0.3.6 added the complete binary notice set; v0.3.7 added automatic takeover of a busy direct output; v0.3.8 made a manual update check open the update dialog and install itself when idle; v0.3.9 sized the ALSA buffer by time (fixing audible gaps at high sample rates); v0.3.10 reaps and restarts a playback daemon that exits while the app is running (`SPOTIFAST-LICENSE`, both generated dependency bundles and the receipt) and updates rustls, quinn-proto and the bundled CA data. The remaining rustls-webpki advisory has no fixed stable release yet.

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
