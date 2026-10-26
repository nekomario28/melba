#!/usr/bin/env python3
"""Keep the English/Japanese front doors and the installer pin in sync."""
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = re.compile(
    r'https://raw\.githubusercontent\.com/nekomario28/melba-wrapper-bootstrap/([^/]+)/install\.sh'
)


def read(name):
    return (ROOT / name).read_text()


class ReadmeDocs(unittest.TestCase):
    def test_language_links(self):
        self.assertTrue('[日本語](README.ja.md)' in read('README.md') or 'href="README.ja.md"' in read('README.md'))
        self.assertIn('[English](README.md)', read('README.ja.md'))

    def test_installer_pin_is_a_tag_and_matches_both_languages(self):
        en = set(INSTALLER.findall(read('README.md')))
        ja = set(INSTALLER.findall(read('README.ja.md')))
        self.assertEqual(en, ja)
        self.assertEqual(len(en), 1)
        tag = en.pop()
        self.assertNotIn(tag, ('main', 'master', 'HEAD'))
        self.assertRegex(tag, r'^v\d')

    def test_documented_catalogs_match_the_bundled_po_files(self):
        bundled = {p.stem for p in (ROOT / 'crates/melba/assets/i18n').glob('*.po')}
        text = read('README.md')
        documented = set(re.findall(r'\b(?:de-DE|es|fr|it|ja|ko|nl|pl|pt-BR|pt-PT|ru|sv|tr|uk|zh-Hans|zh-Hant)\b', text))
        self.assertEqual(documented, bundled)

    def test_wrapper_docs_have_both_languages(self):
        self.assertIn('[日本語](wrapper.ja.md)', (ROOT / 'docs/wrapper.md').read_text())
        self.assertIn('[English](wrapper.md)', (ROOT / 'docs/wrapper.ja.md').read_text())

    def test_project_map_is_reproducible_and_referenced(self):
        with tempfile.TemporaryDirectory(prefix='melba-map-') as folder:
            generated = Path(folder) / 'project-map.svg'
            subprocess.run(
                [sys.executable, str(ROOT / 'docs/assets/project-map.py'), str(generated)],
                check=True,
            )
            self.assertEqual(
                generated.read_bytes(),
                (ROOT / 'docs/assets/project-map.svg').read_bytes(),
            )
        svg = (ROOT / 'docs/assets/project-map.svg').read_text()
        self.assertIn('<title', svg)
        self.assertIn('BITPERFECT', svg)
        self.assertIn('docs/assets/project-map.svg', read('README.md'))
        self.assertIn('docs/assets/project-map.svg', read('README.ja.md'))

    def test_internal_roadmap_is_not_the_front_door(self):
        self.assertNotIn('## Roadmap / TODO', read('README.md'))
        self.assertTrue((ROOT / 'docs/roadmap.md').is_file())


if __name__ == '__main__':
    unittest.main()
