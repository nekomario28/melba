# Installation and packaging

[README](../README.md) · [日本語](../README.ja.md) · [Updates](updates.md)

Melba comprises a desktop app (`melba`) and a playback daemon (`melbad`). Source installation installs both; the default portable layout is compatible with the signed GUI updater.

## Requirements

The toolchain and development headers below are required **only for source builds**, not for the signed prebuilt installer. The prebuilt installer needs `curl`, `openssl`, `tar`, `sha256sum` and standard Unix tools; it provisions known missing runtime libraries on supported distributions. ALAC staging tools are checked separately and installed only if ALAC is selected. `sudo` is requested only for missing system packages.

- Linux x86_64 or aarch64.
- Rust 1.98+ and Cargo; Python 3.9+.
- C/C++ toolchain, CMake, `pkg-config`, and development headers for D-Bus, ALSA and WebKitGTK 4.1 (used by the `auth open` webview).
- `xdg-utils` for browser sign-in. The egui app loads Wayland, X11, xkbcommon and GL at runtime; those are not additional build-time development packages.
- `cargo-about` 0.8.4 for the source-backed third-party notice collector.

The recommended source installer **automatically installs missing dependencies** on CachyOS/Arch, Debian/Ubuntu and Fedora (via the native package manager and `sudo` when necessary). An existing working toolchain is reused. For manual setup, the equivalent system packages are:

```sh
# Debian / Ubuntu
sudo apt install build-essential cmake pkg-config libdbus-1-dev libasound2-dev libwebkit2gtk-4.1-dev xdg-utils python3

# Fedora
sudo dnf install gcc-c++ cmake pkgconf-pkg-config dbus-devel alsa-lib-devel webkit2gtk4.1-devel xdg-utils python3

# Arch
sudo pacman -S --needed base-devel cmake pkgconf dbus alsa-lib webkit2gtk-4.1 xdg-utils python
```

For manual setup only, initialize Rust and install the license collector:

```sh
rustup default stable
cargo install cargo-about --version 0.8.4 --locked
```

Firefox session import also needs the `sqlite3` CLI (`sqlite3` on Debian/Ubuntu; `sqlite` on Fedora/Arch). For AAC subscription playback, a usable Widevine CDM must already be installed; Melba does not download it.

## Distro compatibility of the published v0.3.20 release

The signed portable archive needs **glibc 2.35** or newer; measured on the
published x86_64 archives, `melba` requires at most `GLIBC_2.35` and `melbad`
at most `GLIBC_2.34`. CI builds the candidates in the Ubuntu 22.04 baseline and
then runs the **actual** unsigned candidates in real containers, and runs the
signed Release installer in five of them (without real desktop/audio or Apple
sign-in):

| Distro | glibc | Candidate executes | Signed installer |
| --- | --- | --- | --- |
| Debian 12 (bookworm) | 2.36 | Yes | Yes |
| Debian 13 (trixie) | 2.41 | Yes | — |
| Ubuntu 22.04 LTS | 2.35 | Yes | Yes |
| Ubuntu 24.04 LTS | 2.39 | Yes | Yes |
| Ubuntu 26.04 LTS | 2.43 | Yes | — |
| Fedora 43 | 2.42 | Yes | Yes |
| Fedora 44 | 2.43 | Yes | — |
| Current Arch/CachyOS | 2.43+ | Yes | Yes |

“Yes” under *Candidate executes* means both binaries started and reported
`0.3.13`; “Yes” under *Signed installer* means `packaging/install-release.sh`
downloaded, signature- and checksum-verified and installed the published
release there. This is launch and shared-library compatibility only — not Apple
sign-in, desktop rendering, physical audio output or ALAC playback.

v0.3.12 and older need glibc 2.43, so on Debian 12/13, Ubuntu 22.04/24.04 and
Fedora 43 they must be replaced with v0.3.13 or newer (or built from source). For
distros older than glibc 2.35 use the [source installer](#source-install-developers);
do **not** attempt to upgrade glibc manually to run an application. NixOS uses
the [Nix package](#nix-and-nixos), not the generic portable installer.

## Recommended: signed prebuilt release

On Linux `x86_64` or `aarch64` with **glibc 2.35 or newer** (Debian 12+, Ubuntu 22.04+, Fedora 43+, current Arch/CachyOS), skip the Rust toolchain and Git checkout. The published v0.3.13 binaries need glibc 2.35; older distributions use the [source installer](#source-install-developers) instead. Adding system runtime packages does not upgrade the glibc ABI:

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba/main/packaging/install-release.sh | sh
```

The standalone installer selects the latest matching published release, verifies the Ed25519 signature on its checksum manifest against the embedded Melba publisher key, verifies the archive's SHA-256, then installs the paired `melba`/`melbad` portable layout to `~/.local/opt/melba`. It creates `~/.local/bin` links and a desktop entry (using the publisher-signed Melba icon in new archives, with a generic system-theme fallback for older ones). It works without Cargo or `cargo-about`. It uses `curl`, `openssl`, `tar`, `sha256sum` and standard Unix tools; if ALAC is selected, missing `bash`, `unzip`, `file`, `cc` and related helpers are provisioned as needed. On interactive installs, ALAC is optional with a default-yes `[Y/n]` question; non-interactive installs skip it unless `--with-alac` is explicitly passed. You can also use `--no-alac`. No external wrapper is bundled in the release.

### What the one-command installer does not cover

- **Its own prerequisites.** It uses `curl`, `openssl`, `tar`, `sha256sum` and
  standard Unix tools, and stops with `missing '<tool>'` when one is absent. It
  never installs those.
- **Distributions without `apt-get`, `dnf` or `pacman`.** Missing runtime
  libraries are only provisioned through those three; anything else stops with
  `unsupported distro`. Image-based systems (Fedora Silverblue, openSUSE
  MicroOS) and musl-based ones (Alpine) are **not supported** by this path, and
  no musl build has been verified — use a source build inside a container, or
  the paired Nix package on NixOS.
- **CPUs other than x86_64 and aarch64**, which stop with
  `no prebuilt Melba release for this CPU`.
- **Subscription playback by itself.** Two components are never in the release:
  AAC-256 needs a Widevine CDM already installed by Chrome, Chromium or Vivaldi
  (or `cdm_path` in the config), and ALAC needs a FairPlay decrypt service the
  user supplies. Without the CDM, `stream:` tracks fall back to the 30-second
  catalog preview. Check both with `melbad doctor`.
- **ALAC.** It is never bundled. The installer can stage a third-party service
  (opt-in `[Y/n]`, or `--with-alac` when non-interactive), and `melbad wrapper
  setup` adds it later.

To add ALAC at any time after installation:

```sh
melbad wrapper setup
```

**Version note:** `melbad` checks `bash`, `curl`, `unzip`, `file` and a C compiler (`cc`) and provisions missing tools through pacman, apt-get or dnf. The published **v0.3.13** binary includes this late-setup preflight; **v0.3.12 does not**. On v0.3.12, if these tools are missing, re-run the signed installer with explicit ALAC opt-in (this reuses the published binary and installs missing ALAC tools):

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba/main/packaging/install-release.sh | sh -s -- --with-alac
```

Set `MELBA_INSTALL_AUTO_DEPS=0` to disable automatic package changes. This remains an explicit opt-in; sign in via the GUI. Staging alone is not proof of working ALAC playback. Review the [external runtime terms](wrapper.md).

## Source install (developers)

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

By default, the installer builds `melba` and `melbad`, installs a portable copy under `~/.local/opt/melba`, creates stable links in `~/.local/bin`, and adds a desktop entry. Make sure `~/.local/bin` is on your `PATH`. When invoked from fish, the installer sets a persistent fish universal path automatically; otherwise use `fish_add_path ~/.local/bin` if needed. On supported distributions the installer provisions only missing OS build packages, initializes an existing `rustup` with `rustup default stable` when no working toolchain exists, bootstraps Rust via the official rustup-init HTTPS installer if absent, and installs `cargo-about` when missing. Set `MELBA_INSTALL_AUTO_DEPS=0` to disable this provisioning and fail with an actionable error instead. The source installer offers the optional ALAC service only after an explicit default-yes `[Y/n]` confirmation on a terminal; `--no-alac` skips it and `--with-alac` opts in non-interactively. A companion user unit may be installed, but is not enabled automatically; the desktop starts its own daemon when needed. The installation contains `melba-portable.txt`, required by the GUI updater, plus the project and third-party notices under `share/licenses/melba/`.

Release archives carry `NOTICE.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `SPOTIFAST-LICENSE`, the generated `melba-THIRD-PARTY.txt` / `melbad-THIRD-PARTY.txt` and `receipt.json`. `packaging/package-portable.sh` refuses to package without the generated notices.

Installer variants:

```sh
sh packaging/install.sh               # portable (recommended)
sh packaging/install.sh --no-unit     # skip the optional user unit
sh packaging/install.sh --portable-dir DIR
sh packaging/install.sh --cargo       # developer Cargo-root layout; no GUI updater
sh packaging/install.sh --no-alac    # install Melba only
sh packaging/install.sh --with-alac  # explicitly add ALAC after installing
```

Re-run the installer after `git pull` to update a source installation. The portable installer stages both binaries and restores the previous pair if their versions do not agree. It does not start playback or enable the standalone service.

## Enable ALAC (lossless)

ALAC is optional and is **never bundled** with Melba. If explicitly selected during an interactive install (`[Y/n]`) or using `--with-alac`, the installer stages a separately supplied decrypt service. You can also add it later **without rebuilding or cloning**:

```sh
melbad wrapper setup
melbad wrapper status
```

For builds that include the new preflight, `melbad wrapper setup` provisions missing staging tools on supported distributions and invokes its included setup script. For the already-published v0.3.12 binary, use the signed installer with `--with-alac` as shown above if tools are missing. The third-party runtime has separate Apple/Android library terms; inspect the [independent bootstrap source](https://github.com/nekomario28/melba-wrapper-bootstrap) and security notes before opting in.

**Alternative: run the independent bootstrap yourself.** Use this if you want a checkout to inspect or customize. This method needs `git`, `curl`, `unzip`, `file`, and `cc`, but not Cargo or `cargo-about` when Melba is already installed:

```sh
git clone https://github.com/nekomario28/melba-wrapper-bootstrap.git
cd melba-wrapper-bootstrap
./install.sh --wrapper-only
melbad wrapper status
```

**Alternative: combined source installer from the external bootstrap.** This is for developers who explicitly want a source build and wrapper staging rather than the signed Release path:

```sh
git clone https://github.com/nekomario28/melba-wrapper-bootstrap.git
cd melba-wrapper-bootstrap
./install.sh
```

For reference, the **v1.2.3 bootstrap entry script** is available at https://raw.githubusercontent.com/nekomario28/melba-wrapper-bootstrap/v1.2.3/install.sh. Clone the repository for the documented workflow above: the pinned standalone entry script may resolve its supporting files from the bootstrap repository's current default branch, so the URL alone does not pin the entire operation. Inspect the scripts and external runtime sources before proceeding.

The bootstrap does not use `sudo`, install system packages, accept Apple's terms, handle account credentials or redistribute the external payload as part of Melba. Staging alone does **not** establish a working ALAC stream.

1. Launch Melba, select **Sign in with Apple Music** and complete the GUI sign-in (and MFA if requested). No separate CLI password step is required for the supported managed service.
2. Play a track offered in ALAC. The staged managed service starts on demand; no `wrapper_mode` configuration edit is necessary.
3. If the track falls back to AAC, inspect `melbad wrapper status` and `melbad doctor`. A stopped service before first playback may be normal.

Other decrypt services may use `wrapper_url`, but the GUI native login bridge is only established for the supported managed service. See [Wrapper control](wrapper.md), [Authentication](authentication.md) and [Audio and playback](audio.md). A lossless source does not imply bit-perfect output.

## Other packaging paths

### Arch PKGBUILD

```sh
cd packaging
makepkg -si
```

This VCS recipe builds the repository's **remote default branch**, not necessarily your local checkout. It uses an explicit makepkg source alias (`melba::git+…`) so the downloaded checkout lands in `$srcdir/melba`, which its prepare/build/check/package hooks use. It installs the binaries under `/usr/bin` and a user unit under `/usr/lib/systemd/user`. Only enable the unit if you want a standalone daemon independent of the desktop's lifetime. `makepkg -si` by itself is not a system package repository; to receive VCS package upgrades automatically, publish/subscribe to an AUR package and use an AUR helper such as `paru` or `yay`. Plain `pacman -Syu` does not update AUR packages.

### Nix and NixOS

The normal Linux installer relies on conventional ELF loaders and `apt-get`, `dnf` or `pacman`; **do not use `install-release.sh` directly on NixOS**. It does not manage Nix profiles and does not configure `nix-ld`.

For a native Nix-managed installation of **both** `melba` and `melbad`, use the `melba-paired` derivation:

```sh
nix build ./crates/melba#melba-paired
./result/bin/melba --version
./result/bin/melbad --version
nix profile install ./crates/melba#melba-paired
```

This derivation fetches a specific published Release by a pinned SHA-256 from its publisher-signed checksum manifest, uses Nix's `autoPatchelfHook` to rewrite the dynamic loader and library paths, and includes both executables, the desktop entry, the icon and the available third-party notices. Unlike the portable installer, it is upgraded through **Nix**, not the GUI updater. Release versions and fixed hashes are intentionally changed together.

The existing `nix build ./crates/melba#melba` is **GUI-only**; `nix develop ./crates/melba` remains a developer shell.

**ALAC caveat:** the separately acquired third-party service may require an Android compatibility runtime and additional dynamic-loader configuration. Nix packaging of Melba itself does not imply the external ALAC service has been tested on NixOS; do not rely on `melbad wrapper setup` auto-installing packages through a Nix system manager.

### Manual developer install

The default Cargo install layout is not GUI-updatable. For manual installation and notice collection, the equivalent steps are:

```sh
cargo install --path . --bin melbad --locked
cargo install --path crates/melba --locked
cargo fetch --locked
cargo fetch --locked --manifest-path crates/melba/Cargo.toml
./ci/licenses.sh
install -d ~/.cargo/share/licenses/melba
install -m644 target/third-party/*.txt target/third-party/receipt.json ~/.cargo/share/licenses/melba/
install -Dm644 crates/melba/packaging/melba.desktop ~/.local/share/applications/melba.desktop
install -Dm644 crates/melba/packaging/icons/melba.svg ~/.local/share/icons/hicolor/scalable/apps/melba.svg
install -Dm644 packaging/melbad.service ~/.config/systemd/user/melbad.service
systemctl --user daemon-reload
```

The desktop entry and unit in this manual layout may require adjusting paths for the selected Cargo installation root. The normal installer handles those paths for you. Do not enable an independent login player unless intended.

Both the portable installer and Arch packaging enforce generated third-party notices. Missing or stale notices are fatal; see [NOTICE.md](../NOTICE.md). A package or source build never bundles a Widevine binary, Apple client libraries or an ALAC runtime rootfs.

## Updates and proxies

The portable layout supports signed GUI updates; Cargo-root, source, and package-manager installations have different update paths. See [Updates](updates.md) before relying on the GUI button.

`melba` and `melbad` use the environment's `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` variables. There is no separate proxy form in the app.
