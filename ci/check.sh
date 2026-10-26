#!/usr/bin/env bash
# Slice 06: per-commit gates for melbad. Fail-closed: a gate that cannot run in
# this environment reports NOT_RUN/SKIP, never PASS.
# Usage: ci/check.sh [--hardware]   (hardware gates need an idle hw: device)
set -u
cd "$(dirname "$0")/.."
# A MELBA_BIN that names a path that does not exist must not fail every
# binary gate: fall back to whichever built binary exists. Gates still
# fail if no binary is found.
BIN="${MELBA_BIN:-}"
if [ ! -x "$BIN" ]; then
  for cand in "$BIN" ./target/debug/melbad ./target/release/melbad; do
    [ -x "$cand" ] && { BIN="$cand"; break; }
  done
fi
SCHEMA=contracts/sone.collection.v1.schema.json
fails=0; skips=0
say() { printf '%-42s %s\n' "$1" "$2"; }
fail() { say "$1" FAIL; fails=$((fails+1)); }
pass() { say "$1" PASS; }
skip() { say "$1" "SKIP($2)"; skips=$((skips+1)); }

# Shell gates must stop before unverified tools or rejected dependencies are used.
python3 -m unittest discover -s ci -p 'test_*.py' && pass "ci.python" || fail "ci.python"

# 1. build + tests
cargo test --quiet 2>/dev/null && pass "cargo.test" || fail "cargo.test"
# Exercise the models Home and the player bar consume, not just daemon units.
cargo test --locked --quiet --manifest-path crates/melba/Cargo.toml --test music_data \
  && pass "gui.music-data" || fail "gui.music-data"

# 2. importer -> schema contract (all fixtures)
if [ -f "$SCHEMA" ]; then
  for f in fixtures/playlist.xspf fixtures/list.m3u fixtures/itunes.xml; do
    [ -f "$f" ] || { skip "import:$f" "fixture missing"; continue; }
    if $BIN import "$f" --schema "$SCHEMA" >/dev/null 2>/tmp/melba-ci.err; then
      pass "import:$(basename "$f")"
    else
      fail "import:$(basename "$f")"
      sed 's/^/    /' /tmp/melba-ci.err
    fi
  done
  $BIN scan fixtures/lib --schema "$SCHEMA" >/dev/null 2>&1 && pass "scan:fixtures/lib" || fail "scan:fixtures/lib"
else
  skip "import.schema" "contracts/sone.collection.v1.schema.json absent"
fi

# 3. probe must enumerate at least one device on a host with sound
if [ -d /proc/asound ]; then
  $BIN probe 2>/dev/null | grep -qE '^hw:[0-9]' && pass "probe.devices" || fail "probe.devices"
else
  skip "probe.devices" "no /proc/asound"
fi

# 4. hardware playback receipt — only with --hardware (needs exclusive hw:)
if [ "${1:-}" = "--hardware" ]; then
  out=$($BIN play fixtures/96k-24bit.flac --verify 2>/dev/null)
  echo "$out" | grep -q '"verdict": *"BITPERFECT"' && pass "hires.bitperfect" || fail "hires.bitperfect"
  out=$($BIN play fixtures/96k-mono.flac --verify 2>/dev/null)
  echo "$out" | grep -q '"verdict": *"CLAMPED"' && pass "mono.negative" || fail "mono.negative"
else
  skip "hires.bitperfect" "run ci/check.sh --hardware on a host with an idle hw: device"
fi

# 4b. gxkit structural checks (state machine, module graph, protocol, UI layout)
for tool_model in flowgx:transport depsgx:deps protogx:mpris protogx:daemon_api; do
  tool=${tool_model%%:*}; model=${tool_model##*:}
  if command -v "$tool" >/dev/null; then
    (cd ci/models && "$tool" inspect "$model.py") >/tmp/gx-$model.out 2>&1
    if grep -q "no problems" /tmp/gx-$model.out || grep -q "0 errors" /tmp/gx-$model.out; then
      pass "gx.$model"
    else
      fail "gx.$model"; sed 's/^/    /' /tmp/gx-$model.out
    fi
  else
    skip "gx.$model" "$tool not installed"
  fi
done

# 4b2. model drift guard — declared models mirror code; gx only checks
# internal consistency, so every real bus method must appear in mpris.py
# and every lib module in deps.py, or the mirror silently rots behind it.
drift=0
for m in $(grep -oE 'method\("([A-Za-z]+)"' src/player.rs | sed 's/method("//;s/"//' | sort -u); do
  grep -q "\"$m\"" ci/models/mpris.py || { echo "    bus method $m missing from ci/models/mpris.py" >&2; drift=1; }
done
for m in $(grep -oE 'pub mod [a-z0-9_]+' src/lib.rs | awk '{print $3}' | sed 's/;//'); do
  grep -q "\"$m\"" ci/models/deps.py || { echo "    lib module $m missing from ci/models/deps.py" >&2; drift=1; }
done
# Every ApiRequest/ApiResponse variant must be named in daemon_api.py —
# the model file pins UNANSWERED and DEAD_RESPONSES as empty sets, so a
# newly unanswered request or never-emitted response fails here.
for v in $(sed -n '/pub enum ApiRequest/,/^}/p' crates/melba/src/backend.rs \
           | grep -oE '^\s+[A-Z][A-Za-z0-9]+' | tr -d ' ' | sort -u); do
  grep -q "\"$v\"" ci/models/daemon_api.py \
    || { echo "    ApiRequest::$v missing from ci/models/daemon_api.py" >&2; drift=1; }
done
for v in $(sed -n '/pub enum ApiResponse/,/^}/p' crates/melba/src/backend.rs \
           | grep -oE '^\s+[A-Z][A-Za-z0-9]+' | tr -d ' ' | sort -u); do
  grep -q "\"$v\"" ci/models/daemon_api.py \
    || { echo "    ApiResponse::$v missing from ci/models/daemon_api.py" >&2; drift=1; }
done
[ "$drift" -eq 0 ] && pass "gx.drift" || fail "gx.drift"

# 4b3. lock-poison guard — every .lock() must recover via into_inner(),
# a bare .unwrap() reintroduces the poison cascade the mass fix removed
# (7 sites survived that pass exactly because nothing verified zero left)
if python3 - <<'PY' 2>/dev/null
import re, sys
bad = []
for fn in __import__('glob').glob('src/**/*.rs', recursive=True):
    lines = open(fn).read().splitlines()
    for i, l in enumerate(lines):
        if re.search(r"\.lock\(\)\s*$", l) and i + 1 < len(lines) \
                and lines[i + 1].strip().startswith((".unwrap()", ".expect(")):
            bad.append(f"{fn}:{i + 2}")
        if re.search(r"\.lock\(\)\s*\.unwrap\(", l):
            bad.append(f"{fn}:{i + 1}")
print("\n".join(bad))
sys.exit(1 if bad else 0)
PY
then
  pass "lock.poison"
else
  fail "lock.poison"
fi

# 4b4. xdg cwd-relative guard — PathBuf::from(var_os(...).unwrap_or_default())
# turns an unset HOME into a path relative to the launch cwd (queue.json,
# cdm-store, the webview profile, even the managed-wrapper exec dir all
# leaked there). All XDG resolution goes through config::xdg_dir (Option).
if grep -rn 'var_os("[A-Z_]*")\.unwrap_or_default()' src/ 2>/dev/null; then
  fail "xdg.cwd-relative"
else
  pass "xdg.cwd-relative"
fi

# 4c. both Cargo graphs: SPDX policy + original notices + freshness/integrity
if ./ci/licenses.sh >/tmp/melba-licenses.out 2>&1; then
  pass "licenses"
else
  fail "licenses"; sed 's/^/    /' /tmp/melba-licenses.out
fi

# 5. MPRIS smoke — needs a session bus
if [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  $BIN serve fixtures/44k-16bit.flac fixtures/96k-24bit.flac --device auto >/dev/null 2>&1 &
  sp=$!; sleep 1.5
  st=$($BIN ctl status 2>/dev/null)
  [ "$st" = "Playing" ] || [ "$st" = "Stopped" ] && pass "mpris.serve+ctl" || fail "mpris.serve+ctl"
  # setpos must move Position BACKWARD — a fabricated trackid no-ops under
  # the daemon's stale-trackid guard and position only ever grows
  # (regression: ctl sent /org/mpris/.../Track/current, never a real path)
  pre_pos=$($BIN now 2>/dev/null | grep -o '"positionMs": *[0-9]*' | grep -o '[0-9]*')
  $BIN ctl setpos 0.2 >/dev/null 2>&1; sleep 0.4
  pos=$($BIN now 2>/dev/null | grep -o '"positionMs": *[0-9]*' | grep -o '[0-9]*')
  [ -n "$pre_pos" ] && [ -n "$pos" ] && [ "$pos" -lt "$pre_pos" ] \
    && pass "mpris.setpos" || fail "mpris.setpos"
  # stop must halt without advancing the queue (regression: Stop used to
  # share Next's code path and skip to the next track). The 3s fixture may
  # already have auto-advanced, so compare against the index BEFORE stop.
  pre_idx=$($BIN now 2>/dev/null | grep -o '"queueIndex": *[0-9]*' | grep -o '[0-9]*')
  $BIN ctl stop >/dev/null 2>&1; sleep 0.5
  stop_st=$($BIN ctl status 2>/dev/null)
  stop_idx=$($BIN now 2>/dev/null | grep -o '"queueIndex": *[0-9]*' | grep -o '[0-9]*')
  [ "$stop_st" = "Stopped" ] && [ -n "$pre_idx" ] && [ "$stop_idx" = "$pre_idx" ] \
    && pass "mpris.stop-halts" || fail "mpris.stop-halts"
  # duplicate serve must refuse the owned bus name (exit non-zero)
  dup_out=$(timeout 5 $BIN serve fixtures/44k-16bit.flac --device auto 2>&1)
  echo "$dup_out" | grep -q "already owned" && pass "mpris.dup-guard" || fail "mpris.dup-guard"
  kill $sp 2>/dev/null
else
  skip "mpris.serve+ctl" "no session bus"
fi

echo "----"
echo "fails=$fails skips=$skips"
[ "$fails" -eq 0 ]
