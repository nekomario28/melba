#!/usr/bin/env python3
"""Provenance-record integrity, not media ownership or permission adjudication."""
import copy
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def local_file(root, name):
    relative = Path(name)
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('UNSAFE_MATERIAL_PATH')
    for i in range(1, len(relative.parts) + 1):
        if root.joinpath(*relative.parts[:i]).is_symlink():
            raise ValueError('SYMLINK_MATERIAL_PATH')
    return root / relative


def verify_entry(root, entry):
    data = local_file(root, entry['path']).read_bytes()
    if hashlib.sha256(data).hexdigest() != entry['sha256']:
        raise ValueError('MATERIAL_IDENTITY_MISMATCH')
    if 'bytes' in entry and len(data) != entry['bytes']:
        raise ValueError('MATERIAL_SIZE_MISMATCH')
    if 'git_blob' in entry:
        blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        if blob != entry['git_blob']:
            raise ValueError('MATERIAL_BLOB_IDENTITY_MISMATCH')
    if entry.get('category') == 'extracted_strings':
        if (entry['origin_status'] == 'empty_extraction_output') != (len(data) == 0):
            raise ValueError('EXTRACTION_EMPTINESS_MISMATCH')
    if Path(entry['path']).suffix == '.flac':
        if entry.get('category') != 'audio' or entry['audio']['codec'] != 'flac':
            raise ValueError('FLAC_METADATA_MISMATCH')
        # A retained "24bit" filename actually carries 16-bit STREAMINFO.
        # Read the FLAC header, never infer stored depth from the filename.
        if len(data) < 42 or data[:4] != b'fLaC' or data[4] & 127 != 0 or data[5:8] != b'\0\0\x22':
            raise ValueError('FLAC_STREAMINFO_MISSING')
        packed = int.from_bytes(data[18:26], 'big')
        actual = (packed >> 44, ((packed >> 41) & 7) + 1, ((packed >> 36) & 31) + 1)
        recorded = tuple(entry['audio'][k] for k in ('sample_rate', 'channels', 'stored_bits'))
        if actual != recorded:
            raise ValueError('FLAC_METADATA_MISMATCH')
        label = re.search(r'-(\d+)bit\.flac$', entry['path'])
        mismatch = bool(label and int(label.group(1)) != actual[2])
        if mismatch != entry.get('legacy_name_misstates_depth', False):
            raise ValueError('LEGACY_NAME_MISMATCH_NOT_RECORDED')


def verify_alac_recipes(root, ledger):
    """ALAC fixtures must be reproduced by the recorded, still-identical recipe."""
    source = ledger.get('sources', {}).get('alac_recipe') or {}
    path = source.get('path')
    if not path or not source.get('ffmpeg_version'):
        raise ValueError('ALAC_RECIPE_NOT_RECORDED')
    script = local_file(root, path)
    if hashlib.sha256(script.read_bytes()).hexdigest() != source.get('script_sha256'):
        raise ValueError('ALAC_RECIPE_SCRIPT_MISMATCH')
    alac = [r for r in ledger['records']
            if r.get('category') == 'audio' and (r.get('audio') or {}).get('codec') == 'alac']
    if not alac:
        raise ValueError('ALAC_FIXTURES_MISSING')
    for entry in alac:
        if (entry.get('origin_status') != 'synthetic_recipe_reproduced'
                or entry.get('source') != 'alac_recipe'
                or entry.get('unresolved')):
            raise ValueError('ALAC_RECIPE_NOT_RECORDED')
        if (source.get('regenerated_sha256') or {}).get(entry['path']) != entry['sha256']:
            raise ValueError('ALAC_RECIPE_SHA_MISMATCH')


def declared_paths(root):
    paths = {str(p.relative_to(root)) for p in (root / 'fixtures').rglob('*')
             if p.is_file() and p.suffix in {'.m4a', '.flac'}}
    paths.update(str(p.relative_to(root)) for p in (root / 'evidence/native-scan').glob('*.txt'))
    paths.update({'crates/melba/src/testdata/cast/device-key.pk8', 'crates/melba/assets/skins/melba.wsz'})
    return paths


def verify_coverage(root, records):
    names = [r['path'] for r in records]
    if len(names) != len(set(names)) or set(names) != declared_paths(root):
        raise ValueError('MATERIAL_COVERAGE_MISMATCH')


def verify_supports(root, supports):
    # Independent contract: dropping a binding must not turn off its check.
    required = {
        'fixtures/cover.png', 'crates/melba/LICENSE', 'tests/source_format.rs',
        'crates/melba/src/testdata/cast/device.der',
        'crates/melba/src/testdata/cast/device-sha1.der',
        'research/2026-10-15-auth-path-jadx-scan.md',
        'research/2026-10-15-apple-music-linux-route-measurements.md',
    }
    names = [entry['path'] for entry in supports]
    if len(names) != len(set(names)) or set(names) != required:
        raise ValueError('SUPPORT_COVERAGE_MISMATCH')
    for entry in supports:
        verify_entry(root, entry)


class MaterialProvenance(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = json.loads((ROOT / 'materials/provenance.json').read_text())

    def test_registry_covers_the_declared_materials_and_supports(self):
        records = self.ledger['records']
        verify_coverage(ROOT, records)
        verify_supports(ROOT, self.ledger['supporting_files'])
        for entry in records:
            with self.subTest(path=entry['path']):
                verify_entry(ROOT, entry)
        for entry in records:
            self.assertIn(entry['source'], self.ledger['sources'])
            if 'byte_identical_to' in entry:
                if local_file(ROOT, entry['path']).read_bytes() != local_file(ROOT, entry['byte_identical_to']).read_bytes():
                    raise ValueError('MATERIAL_ALIAS_MISMATCH')

    def test_missing_entry_is_not_a_clean_inventory(self):
        with self.assertRaisesRegex(ValueError, 'MATERIAL_COVERAGE_MISMATCH'):
            verify_coverage(ROOT, self.ledger['records'][1:])

    def test_missing_support_is_not_a_clean_binding(self):
        supports = self.ledger['supporting_files']
        for removed in supports:
            with self.subTest(path=removed['path']):
                with self.assertRaisesRegex(ValueError, '^SUPPORT_COVERAGE_MISMATCH$'):
                    verify_supports(ROOT, [e for e in supports if e['path'] != removed['path']])

    def test_changed_artifact_is_rejected_without_printing_its_contents(self):
        with tempfile.TemporaryDirectory(prefix='material-provenance-') as folder:
            root = Path(folder)
            (root / 'fixture.bin').write_bytes(b'original test material')
            entry = {'path': 'fixture.bin', 'sha256': hashlib.sha256(b'original test material').hexdigest()}
            verify_entry(root, entry)
            (root / 'fixture.bin').write_bytes(b'changed test material')
            with self.assertRaisesRegex(ValueError, '^MATERIAL_IDENTITY_MISMATCH$'):
                verify_entry(root, entry)


    def test_alac_fixtures_bind_to_the_recorded_recipe(self):
        verify_alac_recipes(ROOT, self.ledger)
        broken = copy.deepcopy(self.ledger)
        entry = next(r for r in broken['records']
                     if (r.get('audio') or {}).get('codec') == 'alac')
        entry['source'] = 'local_history'
        entry['origin_status'] = 'local_history_recipe_missing'
        with self.assertRaisesRegex(ValueError, '^ALAC_RECIPE_NOT_RECORDED$'):
            verify_alac_recipes(ROOT, broken)
        broken = copy.deepcopy(self.ledger)
        broken['sources']['alac_recipe']['script_sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, '^ALAC_RECIPE_SCRIPT_MISMATCH$'):
            verify_alac_recipes(ROOT, broken)

    def test_flac_depth_guess_is_rejected_by_the_actual_header(self):
        entry = copy.deepcopy(next(r for r in self.ledger['records'] if r['path'] == 'fixtures/176k-24bit.flac'))
        verify_entry(ROOT, entry)
        # The original misreading was 24 from the name, versus 16 in the file.
        # Alter only the claim, not any shared fixture or source file.
        entry['audio']['stored_bits'] = 24 if entry['audio']['stored_bits'] != 24 else 16
        with self.assertRaisesRegex(ValueError, '^FLAC_METADATA_MISMATCH$'):
            verify_entry(ROOT, entry)
        # Changing a metadata label must not skip the actual-header check.
        entry = copy.deepcopy(next(r for r in self.ledger['records'] if r['path'] == 'fixtures/176k-24bit.flac'))
        entry['audio']['codec'] = 'unknown'
        with self.assertRaisesRegex(ValueError, '^FLAC_METADATA_MISMATCH$'):
            verify_entry(ROOT, entry)

    def test_outside_or_linked_material_is_not_read(self):
        with tempfile.TemporaryDirectory(prefix='material-path-') as folder:
            root = Path(folder)
            with self.assertRaisesRegex(ValueError, 'UNSAFE_MATERIAL_PATH'):
                local_file(root, '../outside')
            (root / 'linked').symlink_to(root / 'nonexistent', target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'SYMLINK_MATERIAL_PATH'):
                local_file(root, 'linked/key.pk8')


if __name__ == '__main__':
    unittest.main()
