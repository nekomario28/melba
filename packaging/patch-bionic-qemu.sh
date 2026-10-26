#!/usr/bin/env bash
# patch-bionic-qemu.sh — make an Android-5.x bionic rootfs boot under
# qemu-aarch64-user on x86_64 hosts. Patches locally staged binaries only.
#
# Usage: patch-bionic-qemu.sh ROOTFS_DIR   (idempotent — .bionic-patched stamp)
set -euo pipefail

ROOT="${1:?usage: $0 ROOTFS_DIR}"
STAMP="$ROOT/../.bionic-patched"

# hygiene (always runs): stray regular files under dev/ — e.g. an empty
# `urandom` left by manual debugging — are served by qemu -L instead of the
# real /dev node; the guest then reads EOF for entropy and kills itself
# with SIGKILL right after "[+] starting...". Sockets/dirs stay untouched.
find "$ROOT/dev" -mindepth 1 -maxdepth 1 -type f -delete 2>/dev/null || true

[ -f "$STAMP" ] && { echo "== bionic already patched"; exit 0; }
command -v python3 >/dev/null || { echo "need python3" >&2; exit 1; }
command -v readelf >/dev/null || { echo "need readelf" >&2; exit 1; }
command -v nm >/dev/null || { echo "need nm" >&2; exit 1; }

# qemu-user compat: this Android-5.x bionic deadlocks on the linker's
# g_dl_mutex under qemu-aarch64 — the recursive-mutex owner check reads
# the tid from TLS and never matches the real caller. Two-part fix:
#
#  1) linker64 is a static binary, so LD_PRELOAD can't reach it — NOP its
#     __dl_pthread_mutex_* entry points (load phase is single-threaded).
#  2) libc stays UNPATCHED: mutex_shim.so (packaging/mutex_shim.c —
#     real futex-based locks with syscall gettid ownership, no TLS) is
#     preloaded and interposes every pthread_mutex_* PLT call, so worker
#     threads actually synchronize instead of racing on NOP'd locks.
python3 - "$ROOT" <<'PY'
import struct, subprocess, sys, os

root = sys.argv[1]
linker = f'{root}/system/bin/linker64'

def sym(file, name):
    out = subprocess.run(['nm', file], capture_output=True,
                         text=True).stdout
    for l in out.splitlines():
        p = l.split()
        if len(p) >= 3 and p[2] == name:
            return int(p[0], 16)
    raise SystemExit(f'symbol not found: {name} in {file}')

def segmap(file):
    # nm reports virtual addresses; patching needs the file offset of the
    # LOAD segment containing them (identical only when p_vaddr==p_offset)
    segs = []
    out = subprocess.run(['readelf', '-lW', file], capture_output=True,
                         text=True).stdout
    for l in out.splitlines():
        p = l.split()
        if p and p[0] == 'LOAD':
            off, va, fsz = int(p[1], 16), int(p[2], 16), int(p[4], 16)
            segs.append((va, va + fsz, off))
    return segs

def va2off(segs, va):
    for lo, hi, off in segs:
        if lo <= va < hi:
            return off + (va - lo)
    raise SystemExit(f'va {va:#x} outside LOAD segments')

NOP = struct.pack('<II', 0x52800000, 0xd65f03c0)          # mov w0,#0; ret

def put(path, va, data, segs):
    with open(path, 'r+b') as f:
        f.seek(va2off(segs, va)); f.write(data)

segs = segmap(linker)
for s in ('__dl_pthread_mutex_lock', '__dl_pthread_mutex_unlock'):
    put(linker, sym(linker, s), NOP, segs)

# mutex_shim.so — built from packaging/mutex_shim.c:
#   clang --target=aarch64-linux-gnu -nostdlib -shared -fPIC -O2 \
#     -fno-stack-protector -fuse-ld=lld -Wl,--hash-style=sysv
SHIM_B64 = "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oAMAAAAAAACgAwAAAAAAAI0AAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAKgAAAAEAAAACAAAAAAAAADAEAAAAAAAAMAQAAAAAAABEAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAADgAAAABAAAAAgAAAAAAAAB4BAAAAAAAAHgEAAAAAAAAvAAAAAAAAAAAAAAAAAAAAAgAAAAAAAAAAAAAAAAAAABCAAAAAQAAAAYAAAAAAAAANAUBAAAAAAA0BQAAAAAAAHQEAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAASAAAAAYAAAADAAAAAAAAAKgJAgAAAAAAqAkAAAAAAABgAAAAAAAAAAQAAAAAAAAACAAAAAAAAAAQAAAAAAAAAFEAAAAIAAAAAwAAAAAAAAAICgIAAAAAAAgKAAAAAAAA+AUAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAABgAAAACAAAAAMAAAAAAAAACAoDAAAAAAAICgAAAAAAAACAAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAAZQAAAAEAAAAwAAAAAAAAAAAAAAAAAAAACAoAAAAAAAApAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAABAAAAAAAAAG4AAAACAAAAAAAAAAAAAAAAAAAAAAAAADgKAAAAAAAAgAEAAAAAAAAOAAAACQAAAAgAAAAAAAAAGAAAAAAAAAB2AAAAAwAAAAAAAAAAAAAAAAAAAAAAAAC4CwAAAAAAAIgAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAgAAAAAMAAAAAAAAAAAAAAAAAAAAAAAAAQAwAAAAAAAC+AAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAA=="
open(f'{root}/system/lib64/mutex_shim.so', 'wb').write(
    __import__('base64').b64decode(SHIM_B64))

# serialized property area + /dev/socket + /system/etc/hosts so that
# __system_properties_init has a context and name resolution works
# without netd/dnsproxyd (both absent under qemu-user).
os.makedirs(f'{root}/dev/__properties__', exist_ok=True)
os.makedirs(f'{root}/dev/socket', exist_ok=True)
ps = f'{root}/dev/__properties__/properties_serial'
if not os.path.exists(ps) or os.path.getsize(ps) == 0:
    hdr = struct.pack('<IIII', 112, 0, 0x504f5250, 0xfc6ed0ab) + b'\0' * 112
    trie = struct.pack('<IIIII', 0, 0, 0, 0, 0) + b'\0'
    open(ps, 'wb').write((hdr + trie)[:131072].ljust(131072, b'\0'))
os.makedirs(f'{root}/system/etc', exist_ok=True)
hosts = ['127.0.0.1 localhost']
for h in ('idmsa.apple.com', 'gsa.apple.com', 'setup.icloud.com',
          'bag.itunes.apple.com', 'itunes.apple.com', 'albert.apple.com',
          'gs.apple.com', 'xp.apple.com', 'init.itunes.apple.com',
          'buy.itunes.apple.com', 'play.itunes.apple.com',
          'music.itunes.apple.com'):
    r = subprocess.run(['getent', 'ahostsv4', h],
                       capture_output=True, text=True).stdout.split()
    if r:
        hosts.append(f'{r[0]} {h}')
open(f'{root}/system/etc/hosts', 'w').write('\n'.join(hosts) + '\n')
print('== bionic qemu-user patches + env staged')
PY

touch "$STAMP"
