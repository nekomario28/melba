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
alac=ask
mode=portable
portable_dir=${MELBA_PORTABLE_DIR:-$HOME/.local/opt/melba}

usage() {
    echo "usage: sh packaging/install.sh [--portable|--cargo] [--portable-dir DIR] [--no-unit] [--with-alac|--no-alac]" >&2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-unit) need_unit=0; shift;;
        --with-alac) alac=yes; shift;;
        --no-alac) alac=no; shift;;
        --portable) mode=portable; shift;;
        --cargo) mode=cargo; shift;;
        --portable-dir)
            [ "$#" -ge 2 ] || { echo "option $1 requires a directory" >&2; usage; exit 2; }
            portable_dir=$2; mode=portable; shift 2;;
        *) usage; exit 2;;
    esac
done

# Provision missing OS packages, Rust and cargo-about as necessary.
sh packaging/install-deps.sh
PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export PATH

# Collect notices before replacing any installed executable. No network is
# allowed during collection; Cargo fetch is the explicit acquisition step.
notices=$(mktemp -d)
build_root=
rollback_dir=
trap 'rm -rf "$notices" "$build_root" "$rollback_dir"' 0
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
    install -Dm644 crates/melba/LICENSE         "$install_root/share/licenses/melba/SPOTIFAST-LICENSE"
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
    install -Dm644 crates/melba/LICENSE         "$build_root/share/licenses/melba/SPOTIFAST-LICENSE"

    install_root=$portable_dir
    mkdir -p "$install_root"
    install_root=$(cd "$install_root" && pwd -P)
    final_bin="$install_root"
    # Snapshot the previous pair before swapping either executable. A
    # version-check failure restores both; never leave a new GUI beside an
    # old daemon (or the reverse).
    rollback_dir=$(mktemp -d)
    for artifact in melba melbad melba-portable.txt; do
        if [ -e "$install_root/$artifact" ]; then
            cp -p "$install_root/$artifact" "$rollback_dir/$artifact"
        fi
    done
    install -m755 "$build_root/bin/melba" "$install_root/.melba.new"
    install -m755 "$build_root/bin/melbad" "$install_root/.melbad.new"
    mv -f "$install_root/.melba.new" "$install_root/melba"
    mv -f "$install_root/.melbad.new" "$install_root/melbad"
    printf 'melba-portable-v1\n' > "$install_root/melba-portable.txt"
    gui_version=$("$install_root/melba" --version 2>/dev/null || true)
    daemon_version=$("$install_root/melbad" --version 2>/dev/null || true)
    case "$gui_version" in
        "melba "*) gui_number=${gui_version#melba };;
        *) gui_number=;;
    esac
    if [ -z "$gui_number" ] || [ "$daemon_version" != "melbad $gui_number" ]; then
        echo "portable pair version check failed (melba: '$gui_version', melbad: '$daemon_version'); rolling back both" >&2
        for artifact in melba melbad melba-portable.txt; do
            if [ -e "$rollback_dir/$artifact" ]; then
                cp -p "$rollback_dir/$artifact" "$install_root/$artifact"
            else
                rm -f "$install_root/$artifact"
            fi
        done
        exit 1
    fi
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
    # A child cannot edit its parent's PATH. Fish universal paths propagate
    # to existing and future fish sessions without rewriting config files.
    case "${SHELL:-}" in
        fish|*/fish)
            if command -v fish >/dev/null 2>&1; then
                MELBA_BIN_DIR="$bin_dir" fish -c 'fish_add_path -U -- "$MELBA_BIN_DIR"' ||
                    echo "fish PATH update failed; run: fish_add_path '$bin_dir'" >&2
            fi
            ;;
    esac
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) echo "Note: $bin_dir is not on PATH; add it before running 'melba'." >&2
           echo "  fish: fish_add_path '$bin_dir'" >&2
           echo "  bash/zsh: export PATH=\"$bin_dir:\$PATH\"" >&2;;
    esac
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
echo "== ALAC (optional external decrypt service) =="
if [ "$alac" = ask ]; then
    # /dev/tty also works when install.sh itself was delivered through a pipe.
    if [ -t 2 ] && [ -r /dev/tty ]; then
        echo "ALAC requires a separately downloaded third-party runtime, including"
        echo "Apple/Android libraries governed by their own terms."
        echo "Details: https://github.com/nekomario28/melba-wrapper-bootstrap"
        printf "Install ALAC lossless support? [Y/n] " >/dev/tty
        IFS= read -r reply </dev/tty || reply=
        case "$reply" in n|N|no|No|NO) alac=no;; *) alac=yes;; esac
    else
        echo "Non-interactive: optional ALAC skipped (pass --with-alac to opt in)."
        alac=no
    fi
fi
if [ "$alac" = yes ]; then
    wrapper_dir=${XDG_DATA_HOME:-$HOME/.local/share}/melba/wrapper
    staged=no
    for entry in run wrapper-lite-qemu wrapper-lite wrapper; do
        [ ! -f "$wrapper_dir/$entry" ] || staged=yes
    done
    if [ "$staged" = yes ]; then
        echo "An ALAC service is already staged at $wrapper_dir; leaving it untouched."
    else
        echo "== Staging user-requested ALAC service =="
        if ! "$final_bin/melbad" wrapper setup; then
            echo "Melba is installed, but ALAC staging failed." >&2
            echo "Retry ALAC without rebuilding Melba: '$final_bin/melbad' wrapper setup" >&2
            exit 1
        fi
    fi
    "$final_bin/melbad" wrapper status || true
else
    echo "ALAC skipped. To add it later, run: '$final_bin/melbad' wrapper setup"
fi
echo "This installer does not bundle or automatically accept third-party terms."
