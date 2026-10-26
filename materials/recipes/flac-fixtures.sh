#!/bin/sh
# Regenerate the FLAC fixtures and the standalone cover from synthetic signal.
# No media input, no account, no audio device. Deterministic for the recorded
# ffmpeg version; a different ffmpeg must be reviewed, not silently substituted.
set -eu
cd "$(dirname "$0")/../.."
EXPECTED='ffmpeg version n9.0.2'
ACTUAL=$(ffmpeg -version | head -n 1)
case "$ACTUAL" in
  "$EXPECTED"*) ;;
  *) echo "ffmpeg version differs from the recorded recipe: $ACTUAL" >&2; exit 2 ;;
esac
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

python3 - "$TMP/cover.png" <<'PY'
import struct, sys, zlib
from pathlib import Path
w = h = 128
rows = bytearray()
for y in range(h):
    rows.append(0)
    for x in range(w):
        r = (x * 2) % 256
        g = (y * 2) % 256
        b = ((x // 16 + y // 16) % 2) * 96 + 64
        rows += bytes((r, g, b))
def chunk(kind, data):
    body = kind + data
    return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)
png = (b'\x89PNG\r\n\x1a\n'
       + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
       + chunk(b'IDAT', zlib.compress(bytes(rows), 9))
       + chunk(b'IEND', b''))
Path(sys.argv[1]).write_bytes(png)
PY

stereo16() {
  rate=$1; out=$2; dur=$3
  ffmpeg -v error -nostdin \
    -f lavfi -i "sine=frequency=440:sample_rate=$rate:duration=$dur" \
    -f lavfi -i "sine=frequency=554:sample_rate=$rate:duration=$dur" \
    -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo[a]" \
    -map "[a]" -c:a flac -sample_fmt s16 -threads 1 \
    -flags +bitexact -fflags +bitexact -map_metadata -1 -y "$out"
}

stereo16 44100 fixtures/44k-16bit.flac 3
stereo16 176400 fixtures/176k-24bit.flac 2
# The legacy name is preserved: this file stores 16-bit STREAMINFO.
ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=96000:duration=1" \
  -map 0:a -ac 1 -c:a flac -sample_fmt s16 -threads 1 \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -y fixtures/96k-mono.flac

ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=96000:duration=3" \
  -f lavfi -i "sine=frequency=554:sample_rate=96000:duration=3" \
  -f lavfi -i "anoisesrc=color=white:sample_rate=96000:duration=3:amplitude=0.000002:seed=7" \
  -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo[st];[2:a]pan=stereo|c0=c0|c1=c0[nz];[st][nz]amix=inputs=2:normalize=0[a]" \
  -map "[a]" -c:a flac -sample_fmt s32 -threads 1 \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -y fixtures/96k-24bit.flac

ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=96000:duration=2" \
  -f lavfi -i "sine=frequency=554:sample_rate=96000:duration=2" \
  -f lavfi -i "sine=frequency=660:sample_rate=96000:duration=2" \
  -f lavfi -i "sine=frequency=880:sample_rate=96000:duration=2" \
  -filter_complex "[0:a][1:a][2:a][3:a]join=inputs=4:channel_layout=quad[a]" \
  -map "[a]" -c:a flac -sample_fmt s16 -threads 1 \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -y fixtures/96k-quad.flac

mkdir -p fixtures/lib
cp -f fixtures/96k-24bit.flac fixtures/lib/trackA.flac
cp -f fixtures/44k-16bit.flac fixtures/lib/trackB.flac
cp -f "$TMP/cover.png" fixtures/cover.png

python3 - <<'PY'
import json, struct, subprocess
expected = {
    'fixtures/176k-24bit.flac': (176400, 16, 2, '2.000000'),
    'fixtures/44k-16bit.flac': (44100, 16, 2, '3.000000'),
    'fixtures/96k-24bit.flac': (96000, 24, 2, '3.000000'),
    'fixtures/96k-mono.flac': (96000, 16, 1, '1.000000'),
    'fixtures/96k-quad.flac': (96000, 16, 4, '2.000000'),
    'fixtures/lib/trackA.flac': (96000, 24, 2, '3.000000'),
    'fixtures/lib/trackB.flac': (44100, 16, 2, '3.000000'),
}
for path, (rate, bits, channels, duration) in expected.items():
    out = subprocess.run(['ffprobe', '-v', 'error', '-show_streams', '-show_format', '-of', 'json', path],
                         capture_output=True, text=True, check=True).stdout
    p = json.loads(out); a = next(s for s in p['streams'] if s['codec_type'] == 'audio')
    assert a['codec_name'] == 'flac', (path, a['codec_name'])
    assert (int(a['sample_rate']), int(a['bits_per_raw_sample']), int(a['channels'])) == (rate, bits, channels), path
    assert abs(float(p['format']['duration']) - float(duration)) < 0.001, (path, p['format']['duration'])
assert open('fixtures/lib/trackA.flac', 'rb').read() == open('fixtures/96k-24bit.flac', 'rb').read()
assert open('fixtures/lib/trackB.flac', 'rb').read() == open('fixtures/44k-16bit.flac', 'rb').read()
raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', 'fixtures/96k-24bit.flac',
                      '-f', 's32le', '-acodec', 'pcm_s32le', '-'], capture_output=True, check=True).stdout
samples = struct.unpack('<%di' % (len(raw) // 4), raw)
assert any(s & 0xff00 for s in samples), 'no signal below the 16-bit floor'
print('FLAC_FIXTURES_REPRODUCED_WITH_REQUIRED_FORMATS_AND_24_BIT_PRECISION')
PY
sha256sum fixtures/176k-24bit.flac fixtures/44k-16bit.flac fixtures/96k-24bit.flac fixtures/96k-mono.flac fixtures/96k-quad.flac fixtures/lib/trackA.flac fixtures/lib/trackB.flac fixtures/cover.png
