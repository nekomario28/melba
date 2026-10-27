#!/bin/sh
# Build the two portable release archives with the marker the GUI updater
# requires. Signing is deliberately separate and needs the private key.
#
#   packaging/package-portable.sh OUT_DIR VERSION MELBA_BIN MELBAD_BIN [TARGET] [NOTICES_DIR]
#
# NOTICES_DIR defaults to target/third-party and must contain the generated
# bundles and receipt. Run ./ci/licenses.sh first; packaging fails closed if
# they are missing.
#
# Produces:
#   melba-vVERSION-TARGET.tar.gz   (melba + melbad + marker + all notices)
#   melbad-vVERSION-TARGET.tar.gz  (melbad + notices)
set -eu
[ "$#" -ge 4 ] && [ "$#" -le 6 ] || {
    echo "usage: $0 OUT_DIR VERSION MELBA_BIN MELBAD_BIN [TARGET] [NOTICES_DIR]" >&2
    exit 2
}
out=$1 version=$2 melba_bin=$3 melbad_bin=$4
target=${5:-x86_64-unknown-linux-gnu}
[ -x "$melba_bin" ] || { echo "not executable: $melba_bin" >&2; exit 1; }
[ -x "$melbad_bin" ] || { echo "not executable: $melbad_bin" >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
notices=${6:-${MELBA_NOTICES:-$root/target/third-party}}
for file in melba-THIRD-PARTY.txt melbad-THIRD-PARTY.txt receipt.json; do
    [ -s "$notices/$file" ] || {
        echo "missing $notices/$file; run ./ci/licenses.sh first" >&2
        exit 1
    }
done
[ -s "$root/crates/melba/LICENSE" ] || { echo "missing crates/melba/LICENSE" >&2; exit 1; }
mkdir -p "$out"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' 0
for name in melba melbad; do
    stem="$name-v$version-$target"
    mkdir -p "$stage/$stem"
    if [ "$name" = melba ]; then
        install -m755 "$melba_bin" "$stage/$stem/melba"
        # The GUI updater installs the daemon from this same archive, so the
        # pair cannot be separated by a GUI-only replacement.
        install -m755 "$melbad_bin" "$stage/$stem/melbad"
        printf 'melba-portable-v1\n' > "$stage/$stem/melba-portable.txt"
    else
        install -m755 "$melbad_bin" "$stage/$stem/melbad"
    fi
    install -m644 "$root/NOTICE.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$stage/$stem/"
    install -m644 "$notices/receipt.json" "$stage/$stem/"
    install -m644 "$root/crates/melba/LICENSE" "$stage/$stem/SPOTIFAST-LICENSE"
    if [ "$name" = melba ]; then
        install -m644 "$notices/melba-THIRD-PARTY.txt" "$notices/melbad-THIRD-PARTY.txt" "$stage/$stem/"
    else
        install -m644 "$notices/melbad-THIRD-PARTY.txt" "$stage/$stem/"
    fi
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
