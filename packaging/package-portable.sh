#!/bin/sh
# Build the two portable release archives with the marker the GUI updater
# requires. Signing is deliberately separate and needs the private key.
#
#   packaging/package-portable.sh OUT_DIR VERSION MELBA_BIN MELBAD_BIN [TARGET]
#
# Produces:
#   melba-vVERSION-TARGET.tar.gz   (melba + melba-portable.txt + licenses)
#   melbad-vVERSION-TARGET.tar.gz  (melbad + licenses)
set -eu
[ "$#" -ge 4 ] && [ "$#" -le 5 ] || {
    echo "usage: $0 OUT_DIR VERSION MELBA_BIN MELBAD_BIN [TARGET]" >&2
    exit 2
}
out=$1 version=$2 melba_bin=$3 melbad_bin=$4
target=${5:-x86_64-unknown-linux-gnu}
[ -x "$melba_bin" ] || { echo "not executable: $melba_bin" >&2; exit 1; }
[ -x "$melbad_bin" ] || { echo "not executable: $melbad_bin" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' 0
for name in melba melbad; do
    stem="$name-v$version-$target"
    mkdir -p "$stage/$stem"
    if [ "$name" = melba ]; then
        install -m755 "$melba_bin" "$stage/$stem/melba"
        printf 'melba-portable-v1\n' > "$stage/$stem/melba-portable.txt"
    else
        install -m755 "$melbad_bin" "$stage/$stem/melbad"
    fi
    install -m644 "$root/NOTICE.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$stage/$stem/"
    tar -C "$stage" -czf "$out/$stem.tar.gz" "$stem"
    if [ "$name" = melba ]; then
        tar -tzf "$out/$stem.tar.gz" | grep -qx "$stem/melba-portable.txt" || {
            echo "portable marker missing from $stem.tar.gz" >&2
            exit 1
        }
    fi
done
(cd "$out" && sha256sum melba-v*.tar.gz melbad-v*.tar.gz > checksums.txt)
echo "PACKAGED $out"
