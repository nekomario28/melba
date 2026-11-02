#!/bin/sh
# Source build preflight: install only missing OS, Rust and Cargo prerequisites.
set -eu
auto=${MELBA_INSTALL_AUTO_DEPS:-1}
case "$auto" in 0|1) ;; *) echo "MELBA_INSTALL_AUTO_DEPS must be 0 or 1" >&2; exit 2;; esac
cargo_bin=${CARGO_HOME:-$HOME/.cargo}/bin
[ ! -d "$cargo_bin" ] || PATH="$cargo_bin:$PATH"
export PATH
have() { command -v "$1" >/dev/null 2>&1; }
packages=""
add() { case " $packages " in *" $1 "*) ;; *) packages="$packages $1";; esac; }
fail() {
    echo "Melba prerequisites: $*" >&2
    echo "Fix the prerequisites and retry: sh packaging/install.sh" >&2
    exit 1
}
cc_missing=0; have cc || cc_missing=1
cxx_missing=0; have c++ || cxx_missing=1
cmake_missing=0; have cmake || cmake_missing=1
pkg_missing=0; have pkg-config || pkg_missing=1
python_missing=0; have python3 || python_missing=1
xdg_missing=0; have xdg-open || xdg_missing=1
curl_missing=0; have curl || curl_missing=1
missing_pc=""
if [ "$pkg_missing" -eq 0 ]; then
    for pc in dbus-1 alsa webkit2gtk-4.1; do
        pkg-config --exists "$pc" || missing_pc="$missing_pc $pc"
    done
else
    missing_pc=" dbus-1 alsa webkit2gtk-4.1"
fi

if [ "$cc_missing" -eq 1 ] || [ "$cxx_missing" -eq 1 ] || \
   [ "$cmake_missing" -eq 1 ] || [ "$pkg_missing" -eq 1 ] || \
   [ "$python_missing" -eq 1 ] || [ "$xdg_missing" -eq 1 ] || \
   [ "$curl_missing" -eq 1 ] || [ -n "$missing_pc" ]; then
    echo "== Missing build prerequisites =="
    [ -z "$missing_pc" ] || echo "missing system dev packages for:$missing_pc" >&2
    [ "$xdg_missing" -eq 0 ] || echo "xdg-open not found" >&2
    [ "$auto" -eq 1 ] || fail "automatic installation disabled (MELBA_INSTALL_AUTO_DEPS=0)"
    os_id= os_like=
    if [ -r /etc/os-release ]; then
        . /etc/os-release
        os_id=$ID
        os_like=${ID_LIKE:-}
    fi
    case " $os_id $os_like " in
        *" arch "*|*" cachyos "*|*" manjaro "*) manager=pacman;;
        *" debian "*|*" ubuntu "*) manager=apt-get;;
        *" fedora "*|*" rhel "*) manager=dnf;;
        *) fail "unsupported distro ($os_id); see docs/installation.md";;
    esac
    have "$manager" || fail "$manager not found"
    case "$manager" in
        pacman)
            if [ "$cc_missing" -eq 1 ] || [ "$cxx_missing" -eq 1 ]; then add base-devel; fi
            [ "$cmake_missing" -eq 0 ] || add cmake
            [ "$pkg_missing" -eq 0 ] || add pkgconf
            for pc in $missing_pc; do
                case "$pc" in dbus-1) add dbus;; alsa) add alsa-lib;;
                    webkit2gtk-4.1) add webkit2gtk-4.1;; esac
            done
            [ "$python_missing" -eq 0 ] || add python
            [ "$xdg_missing" -eq 0 ] || add xdg-utils
            [ "$curl_missing" -eq 0 ] || add curl
            ;;
        apt-get)
            if [ "$cc_missing" -eq 1 ] || [ "$cxx_missing" -eq 1 ]; then add build-essential; fi
            [ "$cmake_missing" -eq 0 ] || add cmake
            [ "$pkg_missing" -eq 0 ] || add pkg-config
            for pc in $missing_pc; do
                case "$pc" in dbus-1) add libdbus-1-dev;; alsa) add libasound2-dev;;
                    webkit2gtk-4.1) add libwebkit2gtk-4.1-dev;; esac
            done
            [ "$python_missing" -eq 0 ] || add python3
            [ "$xdg_missing" -eq 0 ] || add xdg-utils
            [ "$curl_missing" -eq 0 ] || add curl
            ;;
        dnf)
            if [ "$cc_missing" -eq 1 ] || [ "$cxx_missing" -eq 1 ]; then add gcc-c++; fi
            [ "$cmake_missing" -eq 0 ] || add cmake
            [ "$pkg_missing" -eq 0 ] || add pkgconf-pkg-config
            for pc in $missing_pc; do
                case "$pc" in dbus-1) add dbus-devel;; alsa) add alsa-lib-devel;;
                    webkit2gtk-4.1) add webkit2gtk4.1-devel;; esac
            done
            [ "$python_missing" -eq 0 ] || add python3
            [ "$xdg_missing" -eq 0 ] || add xdg-utils
            [ "$curl_missing" -eq 0 ] || add curl
            ;;
    esac
    echo "Installing $manager packages:$packages"
    if [ "$(id -u)" -eq 0 ]; then
        case "$manager" in
            pacman) pacman -S --needed --noconfirm $packages;;
            apt-get) apt-get update && apt-get install -y $packages;;
            dnf) dnf install -y $packages;;
        esac
    else
        have sudo || fail "sudo is required for missing packages"
        case "$manager" in
            pacman) sudo pacman -S --needed --noconfirm $packages;;
            apt-get) sudo apt-get update && sudo apt-get install -y $packages;;
            dnf) sudo dnf install -y $packages;;
        esac
    fi
fi
have cc && have c++ && have cmake && have pkg-config && \
have python3 && have xdg-open && have curl || fail "a required build command is still missing"
for pc in dbus-1 alsa webkit2gtk-4.1; do
    pkg-config --exists "$pc" || fail "missing pkg-config module: $pc"
done

if ! have cargo || ! have rustc || ! cargo --version >/dev/null 2>&1 || \
   ! rustc --version >/dev/null 2>&1; then
    [ "$auto" -eq 1 ] || fail "Rust/Cargo not configured"
    if have rustup; then
        echo "== Configure Rust stable toolchain =="
        rustup default stable
    else
        echo "== Install Rust using official rustup-init =="
        tmp=$(mktemp)
        trap 'rm -f "$tmp"' 0
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$tmp"
        sh "$tmp" -y --no-modify-path
        rm -f "$tmp"
        trap - 0
    fi
    PATH="$cargo_bin:$PATH"
    export PATH
fi
have cargo && have rustc && cargo --version >/dev/null 2>&1 && \
rustc --version >/dev/null 2>&1 || fail "Rust/Cargo still unavailable"

version=$(rustc --version | awk '{print $2}')
major=${version%%.*}
minor=${version#*.}
minor=${minor%%.*}
case "$major:$minor" in *[!0-9:]*|:*) fail "invalid rustc version $version";; esac
if [ "$major" -lt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -lt 98 ]; }; then
    [ "$auto" -eq 1 ] && have rustup || fail "Rust 1.98+ required (found $version)"
    echo "== Upgrade Rust stable toolchain =="
    rustup update stable
    rustup default stable
fi
if ! have cargo-about; then
    [ "$auto" -eq 1 ] || fail "cargo-about 0.8.4 missing"
    echo "== Install cargo-about 0.8.4 =="
    cargo install cargo-about --version 0.8.4 --locked
fi
have cargo-about || fail "cargo-about still missing"
echo "== Build prerequisites ready =="
