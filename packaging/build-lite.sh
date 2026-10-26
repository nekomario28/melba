#!/usr/bin/env bash
# Build upstream wrapper-lite (single-port key service, port 12340) from
# source and install it into the managed wrapper rootfs.
#
# Downloads two unmodified upstream artifacts:
#   1. WorldObservationLog/wrapper source (pinned to WRAPPER_COMMIT)
#   2. Android NDK r23b (the toolchain the upstream CMakeLists hardcodes)
# and links against the bionic .so files already staged by
# install-wrapper.sh — run that first.
#
# Usage:  packaging/build-lite.sh
# Result: ~/.local/share/melba/wrapper/rootfs/system/bin/lite
#         ~/.local/share/melba/wrapper/lite-run   (rootless launcher)
# Then:   ./lite-run --login user:pass   (provisions mpl_db)
#         ./lite-run                     (listens on 127.0.0.1:12340)
#         melba config:  wrapper_url = "http://127.0.0.1:12340/key"
set -euo pipefail

WRAPPER_REPO="https://github.com/WorldObservationLog/wrapper"
# commit this script was validated against; bump deliberately
WRAPPER_COMMIT="2fd7e0eaef081f93cf687428b965674937832ba4"
NDK_URL="https://dl.google.com/android/repository/android-ndk-r23b-linux.zip"
NDK_SHA256="c6e97f9c8cfe5b7be0a9e6c15af8e7a179475b7ded23e2d1c1fa0945d6fb4382"

WRAPPER_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/melba/wrapper"
WORK="${XDG_CACHE_HOME:-$HOME/.cache}/melba-wrapper-lite-build"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1" >&2; exit 1; }; }
need git; need cmake; need ninja; need unzip; need wget; need cc

if [ ! -d "$WRAPPER_DIR/rootfs/system/lib64" ]; then
    echo "run packaging/install-wrapper.sh first (rootfs libs are the link input)" >&2
    exit 1
fi

mkdir -p "$WORK"
cd "$WORK"

# --- NDK r23b (cached) ---
if [ ! -d android-ndk-r23b ]; then
    if [ ! -f ndk.zip ]; then
        echo "==> downloading Android NDK r23b (~725MB)"
        wget -q --show-progress "$NDK_URL" -O ndk.zip
    fi
    echo "==> verifying NDK sha256"
    echo "$NDK_SHA256  ndk.zip" | sha256sum -c -
    unzip -q ndk.zip
fi

# --- upstream source ---
if [ ! -d wrapper/.git ]; then
    git clone "$WRAPPER_REPO" wrapper
fi
cd wrapper
git fetch --depth 1 origin "$WRAPPER_COMMIT"
git checkout -q FETCH_HEAD

# upstream links against the .so files in ./rootfs — install-wrapper's
# staged rootfs is the same content; use it so the binary is built
# against the exact libs it will run against.
ln -sfn "$WRAPPER_DIR/rootfs" rootfs
ln -sfn "$WORK/android-ndk-r23b" android-ndk-r23b

# --- build ---
# upstream's deps (cJSON etc.) predate cmake 4's policy floor
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build

# lite lands in <src>/rootfs/system/bin/ — which is the managed rootfs
# through the symlink above; install the host-side rootless launcher
# next to it.
cc -O2 -o "$WRAPPER_DIR/lite-run" wrapper-lite-rootless.c
ln -sf "rootfs/system/lib64/libcurl.so" "$WRAPPER_DIR/rootfs/system/lib64/libcurl.so.4" 2>/dev/null || true
# the symlink above only covers the managed rootfs when rootfs was a
# real dir in the source tree; if the clone carried its own rootfs,
# copy the binary over instead.
if [ ! -f "$WRAPPER_DIR/rootfs/system/bin/lite" ] && [ -f rootfs/system/bin/lite ]; then
    cp rootfs/system/bin/lite "$WRAPPER_DIR/rootfs/system/bin/lite"
fi
[ -f "$WRAPPER_DIR/rootfs/system/lib64/libcurl.so.4" ] || \
    ln -s libcurl.so "$WRAPPER_DIR/rootfs/system/lib64/libcurl.so.4"

echo "==> installed lite + lite-run under $WRAPPER_DIR"
echo "    next: (cd $WRAPPER_DIR && ./lite-run --login user:pass)"
echo "    then: set wrapper_url to http://127.0.0.1:12340/key"
