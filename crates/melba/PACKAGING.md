# Packaging Melba

The Linux product is a **pair**: the desktop executable `melba` and its
playback/sign-in companion `melbad`. Use the repository's
[source installer or Arch package](../../docs/installation.md) for both binaries
and the notice-carrying install layout. Separately published signed release
archives are available for the documented architectures. A source-only GitHub
tag is not itself an installable desktop package; the [paired portable updater](../../docs/updates.md)
uses signature verification and a matching `melba` / `melbad` installation.

## Install layout and lifetime

The installer and PKGBUILD own the concrete paths for executables, desktop
entry, icon, optional standalone systemd user unit and generated license
bundles. Install the binaries together: the desktop resolves a sibling
`melbad` first, then PATH. It starts a missing companion paused and communicates
with it over MPRIS/D-Bus. Sign-in delegates to `melbad auth gui`; the companion
selects the native or browser flow.

Installing does not enable or start an independent login daemon. Ordinary
close/Quit stops the desktop session's playback unless the user explicitly
chooses background mode. See [desktop lifetime](../../docs/audio.md#queue-and-desktop-control)
for the distinction from standalone daemon use.

The app-only Nix flake does not install `melbad` or collect the paired product's
dependency notices. It is not an equivalent notice-carrying package route.
The existing `contrib/omarchy/` files are optional integration assets, not a
claim that the paired installer stages them.

## Licenses and optional runtime components

The desktop crate is **MIT-only**, derived from Spotifast: retain its original
[`LICENSE`](LICENSE). The root daemon's MIT-or-Apache-2.0 choice does not apply
that choice to this crate. Both Cargo graphs, original dependency notices and
asset notices must be collected and checked as described in
[`NOTICE.md`](../../NOTICE.md). Neither an SPDX list nor a green dependency
scan clears proprietary runtime bundles or research screenshots.

Widevine is loaded from the user's existing installation. Wrapper runtime
bundles (rootfs, Apple/Android libraries, APKs) are not part of this source
distribution and must not be included in a redistributable package. Do not
include repository research material without establishing the relevant rights.

## Updates and persistent state

The in-app updater requires an Ed25519 signature over the release checksums,
verified against the committed public key. This is a verifier contract, not
proof of an operational signing/publishing workflow or control of the private
key. The current portable update helper replaces both executables and restores the
previous pair on a failed GUI startup. This was tested on specified release
paths, not every host or package layout. Package and source upgrades must
install both executables together. See [Updates](../../docs/updates.md).

Configuration, playback state and credentials use the XDG locations owned by
the desktop and daemon. Do not package a user's settings, queue, browser
profile, MusicKit token or wrapper account databases. Test installers against
private fixture directories, never live authentication stores.
