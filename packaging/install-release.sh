#!/bin/sh
# Install the latest publisher-signed Linux release without Rust or git.
# Intended for: curl -fsSL .../packaging/install-release.sh | sh
set -eu

alac=ask
while [ "$#" -gt 0 ]; do
    case "$1" in
        --with-alac) alac=yes;;
        --no-alac) alac=no;;
        -h|--help)
            echo "usage: sh install-release.sh [--with-alac|--no-alac]"
            exit 0;;
        *) echo "Unknown option: $1" >&2; exit 2;;
    esac
    shift
done

fail() { echo "Melba release install: $*" >&2; exit 1; }
for tool in curl openssl sha256sum tar awk uname install ln sed mktemp; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing '$tool'"
done
case "$(uname -m)" in
    x86_64) target=x86_64-unknown-linux-gnu;;
    aarch64|arm64) target=aarch64-unknown-linux-gnu;;
    *) fail "no prebuilt Melba release for this CPU; use the source installer";;
esac

# Publisher key is pinned independently of the downloaded manifest.
# This is the PEM form of crates/melba/assets/update-public-key.hex.
work=$(mktemp -d)
trap 'rm -rf "$work"' 0
trap 'exit 1' 1 2 15
cat >"$work/publisher.pem" <<'KEY'
-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAHkfb92Yn6L5S+nADQ8kBEyMD+CLW3sK+FsZjiL4QSRA=
-----END PUBLIC KEY-----
KEY
base=https://github.com/nekomario28/melba/releases/latest/download
echo "== Fetching signed Melba release manifest =="
curl --proto '=https' --tlsv1.2 -fLsS --retry 2 "$base/checksums.txt" -o "$work/checksums.txt"
curl --proto '=https' --tlsv1.2 -fLsS --retry 2 "$base/checksums.txt.sig" -o "$work/checksums.txt.sig"
openssl pkeyutl -verify -rawin -pubin -inkey "$work/publisher.pem" \
    -in "$work/checksums.txt" -sigfile "$work/checksums.txt.sig" >/dev/null ||
    fail "publisher signature verification failed; no binaries installed"

archive=$(awk -v suffix="-$target.tar.gz" '
    $2 ~ /^melba-v[0-9]/ && length($2) > length(suffix) &&
    substr($2,length($2)-length(suffix)+1) == suffix { print $2 }
' "$work/checksums.txt")
case "$archive" in
    *'
'*|"") fail "signed manifest must contain exactly one GUI archive for $target";;
esac
version=${archive#melba-v}
version=${version%-$target.tar.gz}
case "$version" in ""|*[!0-9A-Za-z.-]*) fail "unexpected signed release version";; esac
printf '== Downloading Melba v%s (%s) ==\n' "$version" "$target"
curl --proto '=https' --tlsv1.2 -fLsS --retry 2 \
    "https://github.com/nekomario28/melba/releases/download/v$version/$archive" \
    -o "$work/$archive"
awk -v f="$archive" '$2 == f { print }' "$work/checksums.txt" |
    (cd "$work" && sha256sum -c -) ||
    fail "downloaded archive does not match the signed checksum"

folder=${archive%.tar.gz}
# Reject unexpected paths even when the archive came from a signed release.
tar -tzf "$work/$archive" | while IFS= read -r member; do
    case "$member" in
        "$folder"|"$folder/"|"$folder/"*) ;;
        *) echo "Unexpected path inside release: $member" >&2; exit 1;;
    esac
    case "/$member/" in *"/../"*|*"/./"*) echo "Unsafe archive path" >&2; exit 1;; esac
done || fail "release archive contains invalid paths"
tar -xzf "$work/$archive" -C "$work"
payload="$work/$folder"
[ -x "$payload/melba" ] && [ -x "$payload/melbad" ] ||
    fail "signed release is missing the paired executables"
[ "$(cat "$payload/melba-portable.txt")" = melba-portable-v1 ] ||
    fail "not a supported portable Melba release"
[ "$("$payload/melba" --version)" = "melba $version" ] ||
    fail "invalid GUI release version"
[ "$("$payload/melbad" --version)" = "melbad $version" ] ||
    fail "invalid daemon release version"

root=${MELBA_PORTABLE_DIR:-$HOME/.local/opt/melba}
mkdir -p "$root"
root=$(cd "$root" && pwd -P)
backup="$work/previous"
mkdir -p "$backup"
for file in melba melbad melba-portable.txt; do
    if [ -f "$root/$file" ]; then cp -p "$root/$file" "$backup/$file"; fi
done

# Swap the pair only after both binaries are verified.
install -m755 "$payload/melba" "$root/.melba.new"
install -m755 "$payload/melbad" "$root/.melbad.new"
mv -f "$root/.melba.new" "$root/melba"
mv -f "$root/.melbad.new" "$root/melbad"
install -m644 "$payload/melba-portable.txt" "$root/melba-portable.txt"
if [ "$("$root/melba" --version 2>/dev/null || true)" != "melba $version" ] ||
   [ "$("$root/melbad" --version 2>/dev/null || true)" != "melbad $version" ]; then
    echo "Installed binary verification failed; restoring previous pair" >&2
    for file in melba melbad melba-portable.txt; do
        if [ -f "$backup/$file" ]; then cp -p "$backup/$file" "$root/$file"
        else rm -f "$root/$file"; fi
    done
    exit 1
fi

for file in NOTICE.md LICENSE-MIT LICENSE-APACHE; do
    [ ! -f "$payload/$file" ] || install -m644 "$payload/$file" "$root/$file"
done
mkdir -p "$root/share/licenses/melba"
for file in SPOTIFAST-LICENSE melba-THIRD-PARTY.txt melbad-THIRD-PARTY.txt receipt.json; do
    [ ! -f "$payload/$file" ] ||
        install -m644 "$payload/$file" "$root/share/licenses/melba/$file"
done

bin_dir=${XDG_BIN_HOME:-$HOME/.local/bin}
mkdir -p "$bin_dir"
ln -sfn "$root/melba" "$bin_dir/melba"
ln -sfn "$root/melbad" "$bin_dir/melbad"
case "${SHELL:-}" in
    fish|*/fish)
        if command -v fish >/dev/null 2>&1; then
            MELBA_BIN_DIR="$bin_dir" fish -c 'fish_add_path -U -- "$MELBA_BIN_DIR"' ||
                echo "PATH hint: fish_add_path '$bin_dir'" >&2
        fi;;
esac

# New signed archives carry the icon; older releases use the system theme.
desktop_icon=multimedia-audio-player
if [ -f "$payload/melba.svg" ]; then
    icon_file="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/melba.svg"
    mkdir -p "$(dirname "$icon_file")"
    install -m644 "$payload/melba.svg" "$icon_file"
    desktop_icon=melba
fi
desktop="${XDG_DATA_HOME:-$HOME/.local/share}/applications/melba.desktop"
mkdir -p "$(dirname "$desktop")"
desktop_exec=$(printf '%s' "$root/melba" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g')
cat >"$desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Melba
GenericName=Music Player
Comment=Apple Music, native and fast
Exec=/usr/bin/env "$desktop_exec" %u
Icon=$desktop_icon
Terminal=false
Categories=AudioVideo;Audio;Music;Player;
StartupWMClass=melba
DESKTOP

echo "Melba $version installed at $root (signed release verified)."
echo "Launch: $root/melba"
echo
echo "== ALAC (optional external decrypt service) =="
if [ "$alac" = ask ]; then
    # Read from the controlling terminal even when invoked via curl | sh.
    if [ -t 2 ] && [ -r /dev/tty ]; then
        echo "ALAC uses a separate third-party runtime with its own terms."
        echo "Details: https://github.com/nekomario28/melba-wrapper-bootstrap"
        printf "Install ALAC support? [Y/n] " >/dev/tty
        IFS= read -r reply </dev/tty || reply=
        case "$reply" in n|N|no|No|NO) alac=no;; *) alac=yes;; esac
    else
        echo "Non-interactive: skipping ALAC (pass --with-alac to opt in)."
        alac=no
    fi
fi
# ALAC needs additional staging tools, not the Rust toolchain.
ensure_alac_tools() {
    missing=""
    for tool in bash curl unzip file cc; do
        command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
    done
    [ -n "$missing" ] || return 0
    [ "${MELBA_INSTALL_AUTO_DEPS:-1}" != "0" ] ||
        fail "missing ALAC prerequisites:$missing (auto provisioning disabled)"
    os_release=${MELBA_INSTALL_OS_RELEASE:-/etc/os-release}
    os_id= os_like=
    if [ -r "$os_release" ]; then
        os_id=$(sed -n 's/^ID=//p' "$os_release" | sed -n '1p' | tr -d '"')
        os_like=$(sed -n 's/^ID_LIKE=//p' "$os_release" | sed -n '1p' | tr -d '"')
    fi
    case " $os_id $os_like " in
        *" arch "*|*" cachyos "*|*" manjaro "*) manager=pacman;;
        *" debian "*|*" ubuntu "*) manager=apt-get;;
        *" fedora "*|*" rhel "*) manager=dnf;;
        *) fail "unsupported distro for missing ALAC tools:$missing";;
    esac
    packages=""
    for tool in $missing; do
        if [ "$tool" = "cc" ]; then
            case "$manager" in
                pacman) package=base-devel;;
                apt-get) package=build-essential;;
                dnf) package=gcc;;
            esac
        else
            package=$tool
        fi
        case " $packages " in
            *" $package "*) ;;
            *) packages="$packages $package";;
        esac
    done
    echo "== Installing missing ALAC tools via $manager:$packages =="
    if [ "$(id -u)" -eq 0 ]; then
        case "$manager" in
            pacman) pacman -S --needed --noconfirm $packages;;
            apt-get) apt-get update && apt-get install -y $packages;;
            dnf) dnf install -y $packages;;
        esac
    else
        command -v sudo >/dev/null 2>&1 || fail "sudo required for ALAC dependencies"
        case "$manager" in
            pacman) sudo pacman -S --needed --noconfirm $packages;;
            apt-get) sudo apt-get update && sudo apt-get install -y $packages;;
            dnf) sudo dnf install -y $packages;;
        esac
    fi
    for tool in bash curl unzip file cc; do
        command -v "$tool" >/dev/null 2>&1 || fail "missing ALAC prerequisite: $tool"
    done
}

if [ "$alac" = yes ]; then
    wrapper_dir="${XDG_DATA_HOME:-$HOME/.local/share}/melba/wrapper"
    staged=no
    for entry in run wrapper-lite-qemu wrapper-lite wrapper; do
        [ ! -f "$wrapper_dir/$entry" ] || staged=yes
    done
    if [ "$staged" = yes ]; then
        echo "ALAC wrapper already present; no changes."
    else
        echo "Staging user-requested ALAC service."
        ensure_alac_tools
        "$root/melbad" wrapper setup ||
            fail "Melba installed successfully but optional ALAC setup failed; retry: $root/melbad wrapper setup"
    fi
    "$root/melbad" wrapper status || true
else
    echo "ALAC skipped. Install later with: $root/melbad wrapper setup"
fi
