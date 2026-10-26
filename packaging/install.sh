#!/bin/sh
# Install melba + melbad from this checkout.
#
# Default: a portable installation under ~/.local/opt/melba, with
# melba-portable.txt, desktop entry and user unit pointing at that directory.
# This is the installation shape the GUI updater accepts. `--cargo` keeps the
# old Cargo-root mode for development; Cargo installations are intentionally
# not GUI-updatable. The desktop owns daemon startup; this script never enables
# an independent login player.
#
#   sh packaging/install.sh                    # portable (recommended)
#   sh packaging/install.sh --no-unit          # portable, no standalone unit
#   sh packaging/install.sh --portable-dir DIR # choose another portable prefix
#   sh packaging/install.sh --cargo            # old Cargo-root developer mode
#
# System prerequisites: see README Install. Check the native headers before
# spending time compiling.

set -eu
cd "$(dirname "$0")/.."

need_unit=1
mode=portable
portable_dir=${MELBA_PORTABLE_DIR:-$HOME/.local/opt/melba}

usage() {
    echo "usage: sh packaging/install.sh [--portable|--cargo] [--portable-dir DIR] [--no-unit]" >&2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-unit) need_unit=0; shift;;
        --portable) mode=portable; shift;;
        --cargo) mode=cargo; shift;;
        --portable-dir)
            [ "$#" -ge 2 ] || { echo "option $1 requires a directory" >&2; usage; exit 2; }
            portable_dir=$2; mode=portable; shift 2;;
        *) usage; exit 2;;
    esac
done

command -v cargo >/dev/null || {
    echo "cargo not found — install a Rust toolchain first" >&2
    exit 1
}
command -v pkg-config >/dev/null || {
    echo "pkg-config not found — see README Install for system deps" >&2
    exit 1
}
missing=""
for pc in dbus-1 alsa webkit2gtk-4.1; do
    pkg-config --exists "$pc" || missing="$missing $pc"
done
if [ -n "$missing" ]; then
    echo "missing system dev packages for:$missing" >&2
    echo "  Debian/Ubuntu: apt install libdbus-1-dev libasound2-dev libwebkit2gtk-4.1-dev" >&2
    echo "  Fedora:        dnf install dbus-devel alsa-lib-devel webkit2gtk4.1-devel" >&2
    echo "  Arch:          pacman -S dbus alsa-lib webkit2gtk-4.1" >&2
    exit 1
fi

command -v xdg-open >/dev/null || {
    echo "xdg-open not found — install xdg-utils for browser sign-in" >&2
    exit 1
}
command -v python3 >/dev/null || {
    echo "python3 not found — needed to collect license notices" >&2
    exit 1
}
command -v cargo-about >/dev/null || {
    echo "cargo-about not found — run: cargo install cargo-about --version 0.8.4 --locked" >&2
    exit 1
}

# Collect notices before replacing any installed executable. No network is
# allowed during collection; Cargo fetch is the explicit acquisition step.
notices=$(mktemp -d)
build_root=
trap 'rm -rf "$notices" "$build_root"' 0
trap 'exit 1' 1 2 15
cargo fetch --locked
cargo fetch --locked --manifest-path crates/melba/Cargo.toml
python3 packaging/licenses.py "$notices"

if [ "$mode" = cargo ]; then
    # Developer mode: install directly into the selected Cargo root, as before.
    install_root=${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}
    mkdir -p "$install_root"
    install_root=$(cd "$install_root" && pwd -P)
    final_bin="$install_root/bin"

    echo "== cargo install: melbad =="
    cargo install --path . --bin melbad --locked --root "$install_root"
    python3 packaging/licenses.py --check "$notices"
    install -Dm644 "$notices/melbad-THIRD-PARTY.txt" \
        "$install_root/share/licenses/melba/melbad-THIRD-PARTY.txt"

    echo "== cargo install: melba =="
    cargo install --path crates/melba --bin melba --locked --root "$install_root"
    python3 packaging/licenses.py --check "$notices"
    for file in melba-THIRD-PARTY.txt receipt.json; do
        install -Dm644 "$notices/$file" "$install_root/share/licenses/melba/$file"
    done
else
    # Portable mode: build into a private staging root, then replace the
    # portable binaries and marker as a unit. The marker is what the GUI
    # updater uses; a cargo-installed binary intentionally lacks it.
    build_root=$(mktemp -d)
    echo "== cargo install: melbad (portable staging) =="
    cargo install --path . --bin melbad --locked --root "$build_root"
    python3 packaging/licenses.py --check "$notices"
    install -Dm644 "$notices/melbad-THIRD-PARTY.txt" \
        "$build_root/share/licenses/melba/melbad-THIRD-PARTY.txt"

    echo "== cargo install: melba (portable staging) =="
    cargo install --path crates/melba --bin melba --locked --root "$build_root"
    python3 packaging/licenses.py --check "$notices"
    for file in melba-THIRD-PARTY.txt receipt.json; do
        install -Dm644 "$notices/$file" "$build_root/share/licenses/melba/$file"
    done

    install_root=$portable_dir
    mkdir -p "$install_root"
    install_root=$(cd "$install_root" && pwd -P)
    final_bin="$install_root"
    for binary in melba melbad; do
        install -m755 "$build_root/bin/$binary" "$install_root/.$binary.new"
        mv -f "$install_root/.$binary.new" "$install_root/$binary"
    done
    printf 'melba-portable-v1\n' > "$install_root/melba-portable.txt"
    install -m644 NOTICE.md LICENSE-MIT LICENSE-APACHE "$install_root/"
    install -d "$install_root/share/licenses/melba"
    for file in "$build_root"/share/licenses/melba/*; do
        [ -f "$file" ] && install -Dm644 "$file" "$install_root/share/licenses/melba/$(basename "$file")"
    done

    # Keep command names available without putting symlinks in Cargo's bin.
    bin_dir=${XDG_BIN_HOME:-$HOME/.local/bin}
    mkdir -p "$bin_dir"
    ln -sfn "$install_root/melba" "$bin_dir/melba"
    ln -sfn "$install_root/melbad" "$bin_dir/melbad"
fi

# Exec has two escaping layers: desktop-entry strings, then quoted arguments.
# Finally escape sed's replacement syntax; percent is a field code/specifier.
desktop_exec=$(printf '%s' "$final_bin/melba" | sed 's/\\/\\\\/g; s/["`$]/\\&/g; s/\\/\\\\/g; s/%/%%/g; s/[\\&|]/\\&/g')
echo "== desktop files =="
desktop_file="${XDG_DATA_HOME:-$HOME/.local/share}/applications/melba.desktop"
mkdir -p "$(dirname "$desktop_file")"
sed "s|^Exec=.*|Exec=/usr/bin/env \"$desktop_exec\" %u|" \
    crates/melba/packaging/melba.desktop > "$desktop_file"
install -Dm644 crates/melba/packaging/icons/melba.svg \
    "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/melba.svg"

if [ "$need_unit" = 1 ] && command -v systemctl >/dev/null; then
    echo "== user systemd unit =="
    unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    mkdir -p "$unit_dir"
    unit_exec=$(printf '%s' "$final_bin/melbad" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g; s/\$/$$/g; s/[\\&|]/\\&/g')
    # env keeps user paths in an argument; systemd restricts executable-path syntax.
    sed "s|^ExecStart=.*|ExecStart=/usr/bin/env \"$unit_exec\" serve|" \
        packaging/melbad.service > "$unit_dir/melbad.service"
    # Register support without starting or enabling a standalone login player.
    systemctl --user daemon-reload
elif [ "$need_unit" = 1 ]; then
    echo "systemctl not found — optional standalone unit skipped; desktop startup still works." >&2
fi

echo
echo "Done. Launch '$final_bin/melba' (or the desktop entry) and sign in."
if [ "$mode" = portable ]; then
    echo "Portable marker: $install_root/melba-portable.txt (GUI updates supported)."
else
    echo "Cargo-root mode: GUI self-update is intentionally unavailable; use --portable."
fi
if [ "$need_unit" = 0 ]; then
    echo "No standalone user unit installed; the desktop starts its co-installed daemon."
fi
echo
echo "== wrapper (ALAC, optional) =="
echo "This installer does not install, fetch or bundle a decrypt service."
echo "An already-installed user-supplied service is controlled by Melba:"
echo "  '$final_bin/melbad' wrapper status"
echo "  '$final_bin/melbad' wrapper stop"
echo "  '$final_bin/melbad' setup        # audio output only"
echo "See docs/wrapper.md; the service's terms and Apple Music access are separate."
