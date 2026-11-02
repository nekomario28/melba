#!/usr/bin/env bash
# Prepare the *only* files accepted by the Melba signed portable updater.
# Run in a trusted, manual GitHub Actions release job, never on PR code.
set -euo pipefail
umask 077

if [[ $# -ne 2 ]]; then
  echo "usage: bash ci/prepare_signed_release.sh ASSETS_DIR VERSION" >&2
  exit 2
fi
assets=$1 version=$2
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "invalid release version" >&2
  exit 2
fi
if [[ -z "${MELBA_SIGNING_KEY_PEM:-}" ]]; then
  echo "MELBA_SIGNING_KEY_PEM is absent: no release may be signed or published" >&2
  exit 1
fi
cd "$assets"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  gui="melba-v${version}-${target}"
  daemon="melbad-v${version}-${target}"
  for stem in "$gui" "$daemon"; do
    test -s "$stem.tar.gz" || { echo "missing $stem.tar.gz" >&2; exit 1; }
    tar -tzf "$stem.tar.gz" | while IFS= read -r item; do
      case "$item" in
        "$stem"|"$stem/"|"$stem/"*) ;;
        *) echo "unsafe archive member: $item" >&2; exit 1 ;;
      esac
      case "/$item/" in
        *"/../"*|*"/./"*) echo "unsafe archive traversal: $item" >&2; exit 1 ;;
      esac
    done
    for file in melbad receipt.json NOTICE.md LICENSE-MIT LICENSE-APACHE melbad-THIRD-PARTY.txt; do
      case "$file" in
        melbad) tar -tzf "$stem.tar.gz" | grep -Fxq "$stem/$file" ;;
        receipt.json|NOTICE.md|LICENSE-MIT|LICENSE-APACHE|melbad-THIRD-PARTY.txt)
          tar -xOf "$stem.tar.gz" "$stem/$file" | test -s /dev/stdin ;;
      esac || { echo "missing or empty $file in $stem" >&2; exit 1; }
    done
  done
  test "$(tar -xOf "$gui.tar.gz" "$gui/melba-portable.txt")" = "melba-portable-v1"
  for file in melba melba-THIRD-PARTY.txt SPOTIFAST-LICENSE; do
    if [[ "$file" = melba ]]; then
      tar -tzf "$gui.tar.gz" | grep -Fxq "$gui/$file"
    else
      tar -xOf "$gui.tar.gz" "$gui/$file" | test -s /dev/stdin
    fi || { echo "missing $file in $gui" >&2; exit 1; }
  done
done

# Reject any unexpectedly carried files instead of signing an unreviewed set.
for file in *; do
  case "$file" in
    "melba-v${version}-x86_64-unknown-linux-gnu.tar.gz"|
    "melba-v${version}-aarch64-unknown-linux-gnu.tar.gz"|
    "melbad-v${version}-x86_64-unknown-linux-gnu.tar.gz"|
    "melbad-v${version}-aarch64-unknown-linux-gnu.tar.gz") ;;
    *) echo "unexpected release asset: $file" >&2; exit 1 ;;
  esac
done

key=$(mktemp)
trap 'rm -f "$key"' EXIT
printf '%s\n' "$MELBA_SIGNING_KEY_PEM" > "$key"
chmod 600 "$key"

# The signing key MUST match the public key already trusted by shipped clients.
openssl pkey -in "$key" -pubout -outform DER > public.der
test "$(wc -c < public.der)" -eq 44 || { echo "not an Ed25519 publisher public key" >&2; exit 1; }
pubhex=$(tail -c 32 public.der | od -An -tx1 | tr -d ' \n')
expected=$(tr -d ' \n' < "$GITHUB_WORKSPACE/crates/melba/assets/update-public-key.hex")
[[ "$pubhex" = "$expected" ]] || { echo "signer does not match shipped Melba verification key" >&2; exit 1; }
rm -f public.der

sha256sum \
  "melba-v${version}-x86_64-unknown-linux-gnu.tar.gz" \
  "melbad-v${version}-x86_64-unknown-linux-gnu.tar.gz" \
  "melba-v${version}-aarch64-unknown-linux-gnu.tar.gz" \
  "melbad-v${version}-aarch64-unknown-linux-gnu.tar.gz" > checksums.txt
openssl pkeyutl -sign -rawin -inkey "$key" \
  -in checksums.txt -out checksums.txt.sig
openssl pkey -in "$key" -pubout > update-public-key.pem
openssl pkeyutl -verify -rawin -pubin -inkey update-public-key.pem \
  -in checksums.txt -sigfile checksums.txt.sig
cp "$GITHUB_WORKSPACE/crates/melba/assets/update-public-key.hex" update-public-key.hex
sha256sum -c checksums.txt
echo "SIGNED_RELEASE_ASSETS_READY: 4 verified bundles + checksum manifest + publisher signature"
