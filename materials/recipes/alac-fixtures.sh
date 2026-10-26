#!/bin/sh
# Regenerate the three ALAC fixtures from synthetic signal only.
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
# Deterministic 64x64 RGB cover: checkerboard gradient, no fonts or libraries.
w = h = 64
rows = bytearray()
for y in range(h):
    rows.append(0)
    for x in range(w):
        r, g, b = (x * 4) % 256, (y * 4) % 256, ((x + y) * 2) % 256
        if (x // 16 + y // 16) % 2:
            r, g, b = g, b, r
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

ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=3" \
  -f lavfi -i "sine=frequency=554:sample_rate=44100:duration=3" \
  -i "$TMP/cover.png" \
  -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo[a]" \
  -map "[a]" -map 2:v -c:a alac -sample_fmt s16p -c:v copy -disposition:v attached_pic \
  -threads 1 -flags +bitexact -fflags +bitexact -map_metadata -1 \
  -metadata title="Bit Test" -metadata artist="melbad fixture" -metadata album="Fixtures" \
  -y fixtures/44k-16bit-art.m4a

ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=88200:duration=2" \
  -f lavfi -i "sine=frequency=554:sample_rate=88200:duration=2" \
  -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo[a]" \
  -map "[a]" -c:a alac -sample_fmt s16p -threads 1 \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -y fixtures/88k-16bit.m4a

# 24-bit needs signal below the 16-bit floor so the stored low byte carries
# real 24-bit precision; a fixed-seed noise floor keeps the recipe reproducible.
ffmpeg -v error -nostdin \
  -f lavfi -i "sine=frequency=440:sample_rate=96000:duration=3" \
  -f lavfi -i "sine=frequency=554:sample_rate=96000:duration=3" \
  -f lavfi -i "anoisesrc=color=white:sample_rate=96000:duration=3:amplitude=0.000002:seed=7" \
  -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo[st];[2:a]pan=stereo|c0=c0|c1=c0[nz];[st][nz]amix=inputs=2:normalize=0[a]" \
  -map "[a]" -c:a alac -sample_fmt s32p -threads 1 \
  -flags +bitexact -fflags +bitexact -map_metadata -1 -y fixtures/96k-24bit.m4a

python3 - <<'PY'
import json, struct, subprocess
expected = {
    'fixtures/44k-16bit-art.m4a': (44100, 16, 2, '3.000000', 'png'),
    'fixtures/88k-16bit.m4a': (88200, 16, 2, '2.000000', None),
    'fixtures/96k-24bit.m4a': (96000, 24, 2, '3.000000', None),
}
for path, (rate, bits, channels, duration, picture) in expected.items():
    out = subprocess.run(['ffprobe', '-v', 'error', '-show_streams', '-show_format', '-of', 'json', path],
                         capture_output=True, text=True, check=True).stdout
    probe = json.loads(out)
    audio = next(s for s in probe['streams'] if s['codec_type'] == 'audio')
    assert audio['codec_name'] == 'alac', (path, audio['codec_name'])
    assert (int(audio['sample_rate']), int(audio['bits_per_raw_sample']), audio['channels']) == (rate, bits, channels), path
    assert abs(float(probe['format']['duration']) - float(duration)) < 0.001, (path, probe['format']['duration'])
    if picture:
        video = next(s for s in probe['streams'] if s['codec_type'] == 'video')
        assert video['codec_name'] == picture, path
        assert video['disposition']['attached_pic'] == 1, path
# 24-bit precision must reach below the 16-bit floor; the source-format test
# depends on it and a 16-bit-quantized input would silently pass as "24-bit".
raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', 'fixtures/96k-24bit.m4a',
                      '-f', 's32le', '-acodec', 'pcm_s32le', '-'],
                     capture_output=True, check=True).stdout
samples = struct.unpack('<%di' % (len(raw) // 4), raw)
assert any(s & 0xff00 for s in samples), 'no signal below the 16-bit floor'
print('ALAC_FIXTURES_REPRODUCED_WITH_REQUIRED_FORMATS_AND_24_BIT_PRECISION')
PY
sha256sum fixtures/44k-16bit-art.m4a fixtures/88k-16bit.m4a fixtures/96k-24bit.m4a
