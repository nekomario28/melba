#!/usr/bin/env bash
# Make apt fail instead of waiting forever.
#
# Every job that installs system packages here has at some point sat in
# `apt-get update` while a mirror stopped sending bytes: twice in
# release-install-smoke (cut off by the 15 minute limit) and once in gates,
# which has no limit of its own and sat there for 44 minutes. A socket timeout
# turns that silence into an error the retry wrapper can act on, so the job
# either finishes or says what went wrong.
#
# Idempotent, and safe on an image that has no apt at all (container jobs that
# use dnf never call it, and nothing else should).
set -euo pipefail

conf_dir="${APT_CONF_DIR:-/etc/apt/apt.conf.d}"
if [ ! -d "$conf_dir" ]; then
    echo "ci/apt-transfers: $conf_dir does not exist here; nothing to configure" >&2
    exit 0
fi
conf="$conf_dir/99-ci-transfers"
body='Acquire::Retries "3";
Acquire::http::Timeout "30";
Acquire::https::Timeout "30";'

if [ -w "$conf_dir" ]; then
    printf '%s\n' "$body" > "$conf"
else
    printf '%s\n' "$body" | sudo tee "$conf" >/dev/null
fi
