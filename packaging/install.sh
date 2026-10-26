#!/bin/sh
# Install melba + melbad from this checkout: cargo bins, desktop files,
# and an optional standalone user unit. The desktop owns daemon startup
# and shutdown; installation must not enable an independent login player. Idempotent — safe to re-run after `git pull`.
#
#   sh packaging/install.sh            # full install
#   sh packaging/install.sh --no-unit  # skip the systemd user unit
#
# System prerequisites: see README Install. Check the native headers before
# spending time compiling.

set -eu
cd "$(dirname "$0")/.."

need_unit=1
case "${1:-}" in
    "") ;;
    --no-unit) need_unit=0 ;;
    *) echo "usage: sh packaging/install.sh [--no-unit]" >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || { echo "too many arguments" >&2; exit 2; }

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
trap 'rm -rf "$notices"' 0
trap 'exit 1' 1 2 15
cargo fetch --locked
cargo fetch --locked --manifest-path crates/melba/Cargo.toml
python3 packaging/licenses.py "$notices"

# Select one explicit Cargo destination, even if its bin directory is not on PATH.
install_root=${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}
mkdir -p "$install_root"
install_root=$(cd "$install_root" && pwd -P)

echo "== cargo install: melbad =="
cargo install --path . --bin melbad --locked --root "$install_root"
# Pair each successful binary with its own notices before attempting the next
# build. A failed GUI update must leave the new daemon and old GUI both covered.
python3 packaging/licenses.py --check "$notices"
install -Dm644 "$notices/melbad-THIRD-PARTY.txt" \
    "$install_root/share/licenses/melba/melbad-THIRD-PARTY.txt"

echo "== cargo install: melba =="
cargo install --path crates/melba --bin melba --locked --root "$install_root"

python3 packaging/licenses.py --check "$notices"
for file in melba-THIRD-PARTY.txt receipt.json; do
    install -Dm644 "$notices/$file" "$install_root/share/licenses/melba/$file"
done

# Exec has two escaping layers: desktop-entry strings, then quoted arguments.
# Finally escape sed's replacement syntax; percent is a field code/specifier.
desktop_exec=$(printf '%s' "$install_root/bin/melba" | sed 's/\\/\\\\/g; s/["`$]/\\&/g; s/\\/\\\\/g; s/%/%%/g; s/[\\&|]/\\&/g')
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
    unit_exec=$(printf '%s' "$install_root/bin/melbad" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g; s/\$/$$/g; s/[\\&|]/\\&/g')
    # env keeps user paths in an argument; systemd restricts executable-path syntax.
    sed "s|^ExecStart=.*|ExecStart=/usr/bin/env \"$unit_exec\" serve|" \
        packaging/melbad.service > "$unit_dir/melbad.service"
    # Register standalone service support without starting playback or
    # changing an existing user's explicit enable/disable choice.
    systemctl --user daemon-reload
elif [ "$need_unit" = 1 ]; then
    echo "systemctl not found — optional standalone unit skipped; desktop startup still works." >&2
fi

echo
echo "Done. Launch '$install_root/bin/melba' (or the desktop entry) and sign in."
if [ "$need_unit" = 0 ]; then
    echo "No standalone user unit installed; the desktop starts its co-installed daemon."
fi
echo
echo "== wrapper (ALAC, optional) =="
echo "This installer does not install, fetch or bundle a decrypt service."
echo "An already-installed user-supplied service is controlled by Melba:"
echo "  '$install_root/bin/melbad' wrapper status"
echo "  '$install_root/bin/melbad' wrapper stop"
echo "  '$install_root/bin/melbad' setup        # audio output only"
echo "See docs/wrapper.md; the service's terms and Apple Music access are separate."
