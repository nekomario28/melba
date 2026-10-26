#!/usr/bin/env python3
"""Guards for the publication-material replacement, with no CDM execution."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

import publication_assets as guard

ROOT = Path(__file__).resolve().parents[1]


class PublicationAssets(unittest.TestCase):
    def test_current_materials_match_the_declared_synthetic_set(self):
        self.assertEqual(guard.check_assets(ROOT), [])

    def test_current_sources_have_no_personal_paths(self):
        self.assertEqual(guard.current_path_issues(ROOT), [])

    def test_personal_path_guard_detects_posix_and_windows_without_values(self):
        samples = ['/' + 'home' + '/fixture-person/Music',
                   '/' + 'Users' + '/fixture-person/Music',
                   'C:' + chr(92) + 'Users' + chr(92) + 'fixture-person']
        for value in samples:
            issues = guard.path_issues(value.encode(), 'fixture.txt')
            self.assertEqual(issues, ['PERSONAL_PATH fixture.txt:1'])
            self.assertNotIn(value, str(issues))
        self.assertEqual(guard.path_issues(b'file:///example/music/a.flac', 'fixture'), [])

    def test_replaced_or_mislabelled_image_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix='publication-assets-') as tmp:
            root = Path(tmp)
            folder = root / 'evidence'
            folder.mkdir()
            manifest = json.loads((ROOT / 'evidence/synthetic-illustrations.json').read_text())
            shutil.copy2(ROOT / 'evidence/render_synthetic.py', folder / 'render_synthetic.py')
            for name in manifest['images']:
                shutil.copy2(ROOT / 'evidence' / name, folder / name)
            selected = next(iter(manifest['images']))
            # Supply a different valid PNG, not a corrupt input that any parser rejects.
            other = next(name for name in manifest['images'] if name != selected)
            shutil.copy2(folder / other, folder / selected)
            target = folder / 'synthetic-illustrations.json'
            target.write_text(json.dumps(manifest))
            self.assertIn('ASSET_IDENTITY_MISMATCH ' + selected, guard.check_assets(root))
            manifest['notice'] = 'Unlabelled image'
            target.write_text(json.dumps(manifest))
            self.assertTrue(any(x.startswith('SYNTHETIC_NOTICE_MISSING') for x in guard.check_assets(root)))

    def test_xspf_example_remains_an_absolute_file_uri(self):
        tree = ET.parse(ROOT / 'fixtures/playlist.xspf')
        uri = tree.find('.//{http://xspf.org/ns/0/}location').text
        self.assertEqual(uri, 'file:///example/music/a.flac')

    def test_probe_requires_an_explicit_path_before_any_library_load(self):
        compiler = os.environ.get('CXX', 'c++')
        with tempfile.TemporaryDirectory(prefix='publication-probe-') as tmp:
            folder = Path(tmp)
            stub = folder / 'loader.cpp'
            stub.write_text('''#include <cstdio>
extern "C" void* publication_fake_dlopen(const char*, int) noexcept {
  puts("PUBLICATION_TEST_LIBRARY_LOAD"); return nullptr;
}
extern "C" char* publication_fake_dlerror() noexcept {
  static char message[] = "synthetic loader error"; return message;
}
''')
            binary = folder / 'probe'
            build = subprocess.run([compiler, '-std=c++17',
                '-Ddlopen=publication_fake_dlopen', '-Ddlerror=publication_fake_dlerror',
                '-I', str(ROOT / 'crates/melba-cdm/third_party/cdm-api'),
                str(ROOT / 'crates/melba-cdm/shim/probe.cc'), str(stub), '-ldl', '-o', str(binary)],
                capture_output=True, text=True, timeout=30)
            self.assertEqual(build.returncode, 0, build.stderr)
            for args in ([], ['',], ['one', 'two']):
                result = subprocess.run([str(binary), *args], capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 2)
                self.assertIn('Usage:', result.stderr)
                self.assertNotIn('PUBLICATION_TEST_LIBRARY_LOAD', result.stdout)
            result = subprocess.run([str(binary), str(folder / 'synthetic.so')],
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertIn('PUBLICATION_TEST_LIBRARY_LOAD', result.stdout)


if __name__ == '__main__':
    unittest.main()
