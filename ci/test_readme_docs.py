#!/usr/bin/env python3
"""Keep the English/Japanese front doors and the installer pin in sync."""
import re
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
        self.assertIn('[日本語](README.ja.md)', read('README.md'))
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

    def test_internal_roadmap_is_not_the_front_door(self):
        self.assertNotIn('## Roadmap / TODO', read('README.md'))
        self.assertTrue((ROOT / 'docs/roadmap.md').is_file())


if __name__ == '__main__':
    unittest.main()
