#!/usr/bin/env python3
"""Current-tree material/path guard, not a secret or legal-clearance scanner."""
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
PERSONAL_PATH = re.compile(rb'(?:/(?:home|Users)/[^/\s"\x27`<>\x00]+|[A-Za-z]:[\\/]+Users[\\/]+[^\\/\s"\x27<>\x00]+)')


def png_info(data):
    if not data.startswith(b'\x89PNG\r\n\x1a\n'):
        raise ValueError('PNG_SIGNATURE')
    pos, dimensions, text = 8, None, {}
    while pos < len(data):
        if pos + 12 > len(data):
            raise ValueError('PNG_TRUNCATED')
        size = struct.unpack('>I', data[pos:pos + 4])[0]
        end = pos + 12 + size
        if end > len(data):
            raise ValueError('PNG_TRUNCATED')
        kind, body = data[pos + 4:pos + 8], data[pos + 8:end - 4]
        if zlib.crc32(kind + body) != struct.unpack('>I', data[end - 4:end])[0]:
            raise ValueError('PNG_CRC')
        if kind == b'IHDR':
            dimensions = struct.unpack('>II', body[:8])
        if kind == b'tEXt':
            key, value = body.split(b'\0', 1)
            text[key.decode('latin1')] = value.decode('latin1')
        pos = end
        if kind == b'IEND':
            if pos != len(data) or dimensions is None:
                raise ValueError('PNG_STRUCTURE')
            return dimensions, text
    raise ValueError('PNG_MISSING_IEND')


def check_assets(root):
    folder = root / 'evidence'
    manifest = json.loads((folder / 'synthetic-illustrations.json').read_text())
    issues = []
    generator = hashlib.sha256((folder / 'render_synthetic.py').read_bytes()).hexdigest()
    if generator != manifest['generator_sha256']:
        issues.append('GENERATOR_IDENTITY_MISMATCH')
    if manifest['kind'] != 'synthetic-illustration' or len(manifest['images']) != 16:
        issues.append('SYNTHETIC_SET_MISMATCH')
    for name, spec in manifest['images'].items():
        if Path(name).name != name or not name.endswith('.png'):
            raise ValueError('UNSAFE_ASSET_PATH')
        data = (folder / name).read_bytes()
        if hashlib.sha256(data).hexdigest() != spec['sha256']:
            issues.append('ASSET_IDENTITY_MISMATCH ' + name)
        dimensions, text = png_info(data)
        if dimensions != (spec['width'], spec['height']):
            issues.append('ASSET_DIMENSIONS_MISMATCH ' + name)
        if text.get('MelbaMaterialKind') != 'synthetic-illustration' or text.get('Description') != manifest['notice']:
            issues.append('SYNTHETIC_NOTICE_MISSING ' + name)
    return issues


def path_issues(data, name):
    if b'\0' in data[:8192]:
        return []
    return [f'PERSONAL_PATH {name}:{data.count(bytes([10]), 0, match.start()) + 1}'
            for match in PERSONAL_PATH.finditer(data)]


def current_path_issues(root):
    paths = subprocess.run(['git', '-C', str(root), 'ls-files', '--cached', '--others',
                            '--exclude-standard', '-z'], check=True, capture_output=True).stdout
    issues = []
    for name in set(p.decode('utf-8') for p in paths.split(b'\0') if p):
        # Upstream test fixtures are not local absolute paths. Keep the
        # vendored fastframe source out of the personal-path scan; its tests
        # use a synthetic home-directory fixture.
        if name.startswith('vendor/'):
            continue
        path = root / name
        if path.is_symlink() or not path.is_file():
            continue
        issues.extend(path_issues(path.read_bytes(), name))
    return sorted(issues)


def main():
    try:
        issues = check_assets(ROOT) + current_path_issues(ROOT)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print('PUBLICATION_GUARD_ERROR', type(error).__name__)
        return 2
    for issue in issues:
        print(issue)
    if not issues:
        print('CURRENT_MATERIAL_IDENTITIES_AND_PERSONAL_PATH_GUARD_PASS_NOT_PUBLICATION_CLEARANCE')
    return 2 if issues else 0


if __name__ == '__main__':
    sys.exit(main())
