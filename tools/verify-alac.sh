#!/usr/bin/env bash
# verify-alac.sh <song-id> [seconds]
#
# Sustained-ALAC playback proof for the melbad daemon. Opens stream:<id>,
# polls MPRIS through a window (default: whole track), and reports:
#   codec    — melbad:codec from MPRIS metadata (alac | mp4a)
#   failure  — first Stopped/error position + serve log tail
#   verdict  — PASS only if Playing past <seconds> or reached clean EOS
#
# Usage:
#   tools/verify-alac.sh 1588049004          # whole track
#   tools/verify-alac.sh 1588049004 90       # first 90s only
#
# Diagnosis hooks baked in:
#   - "starts fine, dies ~30-60s" = prefetch key covers only the head
#     segments; the first real-key segment hits the broken decrypt path.
#   - serve stderr lands in $MELBA_SERVE_LOG (default /tmp/melbad-serve.log —
#     start the daemon with `melbad serve ... >$MELBA_SERVE_LOG 2>&1`).

set -u
ID="${1:?usage: verify-alac.sh <song-id> [seconds]}"
LIMIT="${2:-0}"                     # 0 = follow to EOS
LOG="${MELBA_SERVE_LOG:-/tmp/melbad-serve.log}"
BUS='org.mpris.MediaPlayer2.melbad'

mprop() {
    timeout 4 busctl --user get-property "$BUS" /org/mpris/MediaPlayer2 \
        org.mpris.MediaPlayer2.Player "$1" 2>/dev/null
}
field() { grep -oE "$2\" s \"[^\"]+" <<<"$1" | head -1 | sed 's/.*" s "//'; }

melbad() { timeout 8 "$HOME/.cargo/bin/melbad" "$@"; }

echo "open stream:$ID"; log0=$(( $(wc -l < "$LOG" 2>/dev/null || echo 0) + 1 ))
melbad ctl open "stream:$ID" || { echo "FAIL: open rejected"; exit 1; }

# open is async — the playback thread fetches + decrypts init/seg0 in the
# background. Wait for Playing before trusting position/status, or stale
# metadata from the previous track reports a false EOS.
seen_playing=0; t0=$SECONDS
while [ $seen_playing -eq 0 ]; do
    st=$(melbad ctl status | head -1)
    [ "$st" = Playing ] && seen_playing=1
    [ $((SECONDS-t0)) -ge 60 ] && { echo "FAIL: never reached Playing in 60s"; tail -5 "$LOG" 2>/dev/null; exit 1; }
    sleep 2
done

# metadata for THIS track only exists once Playing starts
meta=$(mprop Metadata); codec=$(field "$meta" 'melbad:codec')
title=$(field "$meta" 'xesam:title'); lenus=$(grep -oE 'mpris:length" x [0-9]+' <<<"$meta" | grep -oE '[0-9]+$')
echo "playing: $title"

t0=$SECONDS; lastpos=0; stuck=0; failed=0
while :; do
    st=$(melbad ctl status | head -1)
    pos=$(mprop Position | grep -oE '[0-9]+$'); pos=${pos:-0}
    meta=$(mprop Metadata); codec=${codec:-$(field "$meta" 'melbad:codec')}
    dur=$(( ${lenus:-0} / 1000000 ))
    printf 't+%3ds  %-8s pos=%ds/%ds codec=%s\n' $((SECONDS-t0)) "$st" $((pos/1000000)) "$dur" "${codec:-?}"

    # progress guard: same position for 3 polls while Playing = stalled
    if [ "$st" = Playing ] && [ "$pos" -le "$lastpos" ]; then
        stuck=$((stuck+1)); [ $stuck -ge 3 ] && { echo "FAIL: position stalled at ${pos}us"; failed=1; break; }
    else stuck=0; fi
    lastpos=$pos

    if [ "$st" != Playing ]; then
        # count only log lines written during THIS run
        if [ -f "$LOG" ] && tail -n +"$log0" "$LOG" | grep -q 'end of stream'; then
            echo "EOS at pos=${pos}us"
            [ $((pos/1000000)) -ge 30 ] && break
        fi
        echo "STOPPED — inspect: tail $LOG / wrapper.log"
        failed=1; break
    fi
    [ "$LIMIT" -gt 0 ] && [ $((SECONDS-t0)) -ge "$LIMIT" ] && break
    sleep 5
done

echo "--- serve log tail ---"; tail -8 "$LOG" 2>/dev/null
if [ $failed -eq 0 ] && [ "$codec" = alac ]; then
    echo "PASS: sustained alac playback"
else
    echo "FAIL: codec=${codec:-none} failed=$failed"
    exit 1
fi
