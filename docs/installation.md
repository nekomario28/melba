# Installation and packaging

[README](../README.md) · [日本語](../README.ja.md) · [Updates](updates.md)

Melba comprises a desktop app (`melba`) and a playback daemon (`melbad`). Source installation installs both; the default portable layout is compatible with the signed GUI updater.

## Requirements

- Linux x86_64 or aarch64.
- Rust 1.98+ and Cargo; Python 3.9+.
- C/C++ toolchain, CMake, `pkg-config`, and development headers for D-Bus, ALSA and WebKitGTK 4.1 (used by the `auth open` webview).
- `xdg-utils` for browser sign-in. The egui app loads Wayland, X11, xkbcommon and GL at runtime; those are not additional build-time development packages.
- `cargo-about` 0.8.4 for the source-backed third-party notice collector.

Install distro dependencies (with your normal package manager privileges):

```sh
# Debian / Ubuntu
sudo apt install build-essential cmake pkg-config libdbus-1-dev libasound2-dev libwebkit2gtk-4.1-dev xdg-utils python3

# Fedora
sudo dnf install gcc-c++ cmake pkgconf-pkg-config dbus-devel alsa-lib-devel webkit2gtk4.1-devel xdg-utils python3

# Arch
sudo pacman -S --needed base-devel cmake pkgconf dbus alsa-lib webkit2gtk-4.1 xdg-utils python
```

Install Rust separately, then the build-time license collector:

```sh
cargo install cargo-about --version 0.8.4 --locked
```

Firefox session import also needs the `sqlite3` CLI (`sqlite3` on Debian/Ubuntu; `sqlite` on Fedora/Arch). For AAC subscription playback, a usable Widevine CDM must already be installed; Melba does not download it.

## Recommended source install

```sh
git clone https://github.com/nekomario28/melba.git
cd melba
sh packaging/install.sh
melba
```

By default, the installer builds `melba` and `melbad`, installs a portable copy under `~/.local/opt/melba`, creates stable links in `~/.local/bin`, and adds a desktop entry. Make sure `~/.local/bin` is on your `PATH`. A companion user unit may be installed, but is not enabled automatically; the desktop starts its own daemon when needed. The installation contains `melba-portable.txt`, required by the GUI updater.

Installer variants:

```sh
sh packaging/install.sh               # portable (recommended)
sh packaging/install.sh --no-unit     # skip the optional user unit
sh packaging/install.sh --portable-dir DIR
sh packaging/install.sh --cargo       # developer Cargo-root layout; no GUI updater
```

Re-run the installer after `git pull` to update a source installation. The portable installer stages both binaries and restores the previous pair if their versions do not agree. It does not start playback or enable the standalone service.

## Optional source-only wrapper bootstrap

The separate [melba-wrapper-bootstrap](https://github.com/nekomario28/melba-wrapper-bootstrap) project installs Melba from source if needed and stages an upstream wrapper for local use:

```sh
curl -fsSL https://raw.githubusercontent.com/nekomario28/melba-wrapper-bootstrap/v1.2.3/install.sh | bash
```

Read the script before executing it. System build dependencies above still need to be installed. The bootstrap does not use `sudo`, accept Apple's terms, handle your account credentials or redistribute the wrapper/rootfs as part of Melba. To avoid launching the GUI, add `-s -- --no-launch` to the command. For service handling and supported modes, see [Wrapper](wrapper.md).

The standard Melba installer does **not** install this external service or its runtime.

## Other packaging paths

### Arch PKGBUILD

```sh
cd packaging
makepkg -si
```

This VCS recipe builds the repository's **remote default branch**, not necessarily your local checkout. It installs the binaries under `/usr/bin` and a user unit under `/usr/lib/systemd/user`. Only enable the unit if you want a standalone daemon independent of the desktop's lifetime.

### Nix (app-only)

`crates/melba/flake.nix` offers:

```sh
nix build ./crates/melba#melba
nix develop ./crates/melba
```

The Nix path builds the desktop app only; it does not install `melbad` or its user unit. It has not been verified through the installation/license-notice checks above and does not run the dependency notice collector. Do not treat it as equivalent to the supported paired portable installation.

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
