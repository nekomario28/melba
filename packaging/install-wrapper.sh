#!/usr/bin/env bash
# install-wrapper.sh — stage the managed ALAC decrypt service for melba.
#
# Downloads a SHA-256-pinned WorldObservationLog/wrapper runtime bundle.
# Its rootfs includes Apple and Android shared libraries; fetching it locally
# does not establish permission to use or redistribute those components.
# The wrapper helper's source license does not license its bundled rootfs.
# No APK is downloaded: --apk FILE optionally replaces libraries from a local
# APK/XAPK supplied by the user. See NOTICE.md for the distribution boundary.
# Staging does not execute or authenticate the service. An active Apple Music
# subscription is required for subscription playback.
#
# Usage: install-wrapper.sh [--dir DIR] [--apk FILE] [--arch x86_64]
set -euo pipefail

DIR="${XDG_DATA_HOME:-$HOME/.local/share}/melba/wrapper"
APK=""
ARCH="$(uname -m)"; [ "$ARCH" = "amd64" ] && ARCH=x86_64
WRAPPER_REPO="https://github.com/WorldObservationLog/wrapper"

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) DIR="$2"; shift 2;;
        --apk) APK="$2"; shift 2;;
        --arch) ARCH="$2"; shift 2;;
        *) echo "unknown arg: $1" >&2; exit 2;;
    esac
done

case "$ARCH" in
    x86_64|aarch64|arm64) [ "$ARCH" = "aarch64" ] && ARCH=arm64;;
    *) echo "unsupported arch: $ARCH" >&2; exit 2;;
esac

say(){ printf '== %s\n' "$*"; }
need(){ command -v "$1" >/dev/null || { echo "need $1" >&2; exit 1; }; }
need curl; need unzip; need file

# resolve $0 BEFORE `cd "$DIR"` — a relative invocation path stops
# resolving once we've moved (cd: packaging: No such file or directory)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$DIR"
cd "$DIR"

# ---------- 1. wrapper release ----------
# Upstream ships a self-contained bundle per host arch: rootfs/system/
# lib64 carries the whole Android library stack INCLUDING the Apple .so
# files the service dlopens (libstoreservicescore, libmediaplatform,
# libCoreFP…), and `wrapper` is a tiny glibc launcher that chdir()s into
# rootfs and execs system/bin/main under the bundled bionic linker64.
# x86_64 hosts run the x86_64 bundle Natively — no CPU emulation at all
# (bionic's pthreads/TLS work normally; the earlier arm64+qemu-user
# route is kept in packaging/ for exotic cases but is not the default).
TAG="wrapper.${ARCH}.latest"
ZIP="Wrapper.${ARCH}.latest.zip"
# Upstream publishes only a mutable ".latest" tag, so the integrity anchor
# is the payload itself: these are sha256 of the zips measured on
# 2026-10-19 (x86_64 48,826,944 B / arm64 51,411,847 B). A mismatch means
# upstream re-published (or something interfered) — refuse rather than
# run unverified code in the process an Apple ID login later passes
# through. To adopt a newer upstream build: download it yourself, check
# it, update these two lines (or export MELBA_WRAPPER_SHA256=<hex>).
SHA256_x86_64="4bf5ec7869c57baeab13832d14b5c8d1c1167a6dd492a837f1d9b8fb42c2d67a"
SHA256_arm64="8de5da67e375038ff5a08761c71ccaf90ac5ec711a6d0924ba7328b5e116e0a6"
case "$ARCH" in
    x86_64) DEF_SHA="$SHA256_x86_64";;
    arm64)  DEF_SHA="$SHA256_arm64";;
esac
WANT_SHA="${MELBA_WRAPPER_SHA256:-$DEF_SHA}"
if [ ! -x "$DIR/run" ] && [ ! -x "$DIR/wrapper" ]; then
    say "downloading $ZIP (self-contained $ARCH rootfs)"
    curl -fL "$WRAPPER_REPO/releases/download/$TAG/$ZIP" -o wrapper.zip
    GOT_SHA="$(sha256sum wrapper.zip | cut -d' ' -f1)"
    if [ "$GOT_SHA" != "$WANT_SHA" ]; then
        rm -f wrapper.zip
        echo "checksum mismatch for $ZIP" >&2
        echo "  want $WANT_SHA" >&2
        echo "  got  $GOT_SHA" >&2
        echo "upstream re-published .latest or the download was tampered" >&2
        echo "with; verify the new build, then update the pin or export" >&2
        echo "MELBA_WRAPPER_SHA256=<hex> — refusing to stage unverified code" >&2
        exit 1
    fi
    unzip -o -q wrapper.zip
    rm wrapper.zip
    # only a *nested* wrapper dir needs flattening; ./wrapper itself is
    # already in place and "mv ./* ." would be a noisy no-op
    nested="$(find . -mindepth 2 -maxdepth 2 -name 'wrapper' -type f | head -1 || true)"
    if [ -n "$nested" ] && [ ! -e ./wrapper ]; then
        base="$(dirname "$nested")"
        mv "$base"/* . && rmdir "$base" 2>/dev/null || true
    fi
    chmod +x wrapper wrapper-rootless 2>/dev/null || true
else
    say "wrapper payload already present"
fi

LIBDIR="$DIR/rootfs/system/lib64"
if [ -z "$(ls -A "$LIBDIR"/*.so 2>/dev/null)" ]; then
    echo "release bundle has no lib64 — unexpected upstream layout" >&2
    exit 1
fi

# ---------- 1b. tzdata ----------
# The release zips omit system/usr/share/zoneinfo/tzdata; bionic then
# logs "__bionic_open_tzdata: couldn't find any tzdata" and any guest
# code validating time (TLS cert checks, media expiry) reads wrong
# dates. It's arch-independent bionic-format data tracked in the
# upstream git repo — stage it when absent.
TZD="$DIR/rootfs/system/usr/share/zoneinfo/tzdata"
# tzdata lives only on the upstream "lite" branch — pinned to the exact
# commit (2fd7e0e) and sha256-verified like the release zip.
TZDATA_REF="2fd7e0eaef081f93cf687428b965674937832ba4"
TZDATA_SHA256="28d0689618cfb2796a52ba335ab9e535ba68948ad3ecd1a5670e24f762ff2117"
if [ ! -s "$TZD" ]; then
    say "staging bionic tzdata (missing from release zips)"
    mkdir -p "$(dirname "$TZD")"
    if curl -fsSL "https://raw.githubusercontent.com/WorldObservationLog/wrapper/$TZDATA_REF/rootfs/system/usr/share/zoneinfo/tzdata" \
        -o "$TZD" \
        && [ "$(sha256sum "$TZD" | cut -d' ' -f1)" = "$TZDATA_SHA256" ]; then
        say "tzdata staged (pinned @ ${TZDATA_REF:0:8})"
    else
        rm -f "$TZD"
        echo "warning: pinned tzdata fetch/verify failed — guest TLS" >&2
        echo "         time checks may misbehave; continuing without it" >&2
    fi
fi

# ---------- 1c. argv scrub ----------
# Login execs with `-L @` and the real user:pass in a memfd. This
# freestanding preload repoints that argv slot in memory and strips the
# secret again on every execve, so /proc/<pid>/cmdline never holds it.
# An already-installed .so from before the memfd protocol (no marker)
# must be rebuilt; login refuses to start without the marker. Setup
# fails closed when the shim cannot be built (needs cc; host==target arch).
SCRUB_SRC="$SCRIPT_DIR/argv-scrub.c"
if ! grep -q -a -F 'melba-login-fd-v2' "$LIBDIR/argv-scrub.so" 2>/dev/null; then
    [ -f "$SCRUB_SRC" ] || {
        echo "argv-scrub.c missing next to install-wrapper.sh" >&2
        exit 1
    }
    command -v cc >/dev/null || {
        echo "cc is required to build argv-scrub.so; without it login cannot hand the Apple ID password over without putting it on argv" >&2
        exit 1
    }
    cc -O2 -fPIC -shared -nostdlib -fno-stack-protector -fno-builtin \
        -o "$LIBDIR/argv-scrub.so" "$SCRUB_SRC" \
        -Wl,--hash-style=sysv -Wl,-z,noexecstack \
        || { rm -f "$LIBDIR/argv-scrub.so"; echo "failed to build argv-scrub.so" >&2; exit 1; }
    say "built argv-scrub.so (login credential stays off the command line)"
fi

# ---------- 2. optional APK override ----------
# Not required (the bundle already ships every .so). --apk only replaces
# the bundled libs with a different Apple Music build — e.g. the
# upstream-tested 3.6.0-beta. APK ABI must match the staged rootfs.
if [ -n "$APK" ]; then
    if [ "$ARCH" = x86_64 ]; then ABI=x86_64; EMACH=x86-64
    else ABI=arm64-v8a; EMACH=aarch64
    fi
    # extract into a scratch dir first — the old check counted the
    # bundle's own .so files, so a failed/wrong-ABI extraction still
    # reported success and could half-overwrite lib64.
    stage="$(mktemp -d)"; trap 'rm -rf "$stage" "${tmp:-}"' EXIT
    extract_from() {
        rm -f "$stage"/*.so 2>/dev/null || true
        unzip -o -q -j "$1" "lib/$ABI/*.so" -d "$stage" 2>/dev/null \
            && [ -n "$(ls -A "$stage"/*.so 2>/dev/null)" ]
    }
    if unzip -l "$APK" | grep -q "lib/$ABI/"; then
        say "extracting lib/$ABI/*.so from $APK"
        extract_from "$APK" || { echo "no lib/$ABI found in $APK" >&2; exit 1; }
    else
        say "XAPK bundle — scanning inner split APKs for lib/$ABI"
        tmp="$(mktemp -d)"
        unzip -o -q "$APK" "*.apk" -d "$tmp" 2>/dev/null || true
        ok=""
        for inner in "$tmp"/*.apk; do
            if unzip -l "$inner" 2>/dev/null | grep -q "lib/$ABI/"; then
                say "found lib/$ABI in $(basename "$inner")"
                extract_from "$inner" && { ok=1; break; }
            fi
        done
        [ -n "$ok" ] || { echo "no inner APK carries lib/$ABI" >&2; exit 1; }
    fi
    if file "$stage"/*.so | grep -vq "$EMACH"; then
        echo "APK lib arch mismatch — refusing to overwrite rootfs:" >&2
        file "$stage"/*.so | grep -v "$EMACH" | head -3 >&2
        exit 1
    fi
    cp -f "$stage"/*.so "$LIBDIR"/
    say "replaced $(ls "$stage"/*.so | wc -l) $EMACH libs in rootfs lib64"
fi

# ---------- 3. launcher ----------
mkdir -p "$DIR/rootfs/data/data/com.apple.android.music/files"
cat > "$DIR/run" <<'EOS'
#!/bin/sh
D="$(cd "$(dirname "$0")" && pwd)"
exec "$D/wrapper" -H 127.0.0.1 \
    -B "$D/rootfs/data/data/com.apple.android.music/files" "$@"
EOS
chmod +x "$DIR/run"
say "native $ARCH service staged — no emulation, no binary patches"
say "payload not executed; staging does not verify login, FairPlay or playback"
say "the rootfs includes third-party libraries; review their terms before use or redistribution"
say "first run needs a real Apple ID: melbad wrapper login <user>"
say "ports bind only after a successful login/provisioning pass."

say "done. layout:"
ls "$DIR" | sed 's/^/   /'
echo
echo "set \"wrapper_mode\": \"managed\" in ~/.config/melba/config.json —"
echo "melbad will start the service automatically the first time an ALAC"
echo "track is opened, and leave it stopped when ALAC is off."
echo "first use also needs an Apple ID once — \`melbad wrapper login <user>\`"
echo "(password prompts on the terminal with echo off, or MELBA_WRAPPER_PASSWORD)"
echo "provisions the device/account database;"
echo "afterwards the service runs unattended (2FA via -F/2fa.txt if asked)."
