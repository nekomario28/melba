#!/usr/bin/env bash
# Fetch one GitHub release asset, whichever front door answers, and verify it.
#
# `github.com/<repo>/releases/download/...` has returned 504 for whole hours
# while `api.github.com` served the same bytes — long enough to fail every job
# that downloads a pinned tool, on pull requests unrelated to the outage. The
# direct route is tried first because it is the cheap one; the API is the same
# repository and the same file.
#
# Usage: ci/fetch-release-asset.sh <owner/repo> <tag> <asset> <dest> <sha256>
set -euo pipefail

repo=$1; tag=$2; asset=$3; dest=$4; want=$5
direct="https://github.com/$repo/releases/download/$tag/$asset"
# A tag may carry a slash (`cargo-audit/v0.22.2`), which the API path needs
# encoded.
encoded=$(printf '%s' "$tag" | sed 's|/|%2F|g')
api="https://api.github.com/repos/$repo/releases/tags/$encoded"

if curl -fsSL --retry 3 -o "$dest" "$direct"; then
    :
else
    echo "direct download failed; retrying $asset through the API" >&2
    # The asset id is printed by jq, because matching JSON with awk also
    # matches the `id` of the release object and of every other asset listed
    # before the one wanted.
    id=$(curl -fsSL --retry 3 "$api" |
        python3 -c 'import json,sys; want=sys.argv[1]; print(next((a["id"] for a in json.load(sys.stdin).get("assets", []) if a["name"]==want), ""))' "$asset")
    [ -n "$id" ] || { echo "no asset named $asset in $repo@$tag" >&2; exit 1; }
    curl -fsSL --retry 3 -H 'Accept: application/octet-stream' -o "$dest" \
        "https://api.github.com/repos/$repo/releases/assets/$id"
fi
printf '%s  %s\n' "$want" "$dest" | sha256sum -c --status || {
    echo "downloaded $asset does not match its pinned sha256" >&2
    exit 1
}
