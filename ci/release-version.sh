#!/usr/bin/env bash
# Shared publisher contract: safe stable versions and numbered alpha releases.
set -euo pipefail
[[ $# == 1 ]] || { echo 'usage: release-version.sh VERSION' >&2; exit 2; }
number='(0|[1-9][0-9]*)'
if [[ $1 =~ ^$number\.$number\.$number$ ]]; then
  echo stable
elif [[ $1 =~ ^$number\.$number\.$number-alpha\.$number$ ]]; then
  echo alpha
else
  echo 'invalid release version (expected X.Y.Z or X.Y.Z-alpha.N)' >&2
  exit 2
fi
