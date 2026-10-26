#!/usr/bin/env bash
# One owner for license policy, source-backed notices, and both Cargo graphs.
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 packaging/licenses.py "${1:-target/third-party}"
