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
        self.assertTrue('[日本語](README.ja.md)' in read('README.md') or 'href="README.ja.md"' in read('README.md'))
        self.assertIn('[English](README.md)', read('README.ja.md'))

    def test_installer_pin_is_a_tag_and_matches_both_languages(self):
        # The install details live in docs/installation.md; the front READMEs
        # may link there without duplicating the external command.
        source = read('docs/installation.md')
        tags = set(INSTALLER.findall(source))
        self.assertEqual(len(tags), 1)
        tag = next(iter(tags))
        self.assertNotIn(tag, ('main', 'master', 'HEAD'))
        self.assertRegex(tag, r'^v\d')
        for name in ('README.md', 'README.ja.md'):
            found = set(INSTALLER.findall(read(name)))
            self.assertTrue(not found or found == tags, name)

    def test_documented_catalogs_match_the_bundled_po_files(self):
        bundled = {p.stem for p in (ROOT / 'crates/melba/assets/i18n').glob('*.po')}
        text = read('docs/development.md')
        self.assertIn('docs/development.md', read('README.md'))
        documented = set(re.findall(r'\b(?:de-DE|es|fr|it|ja|ko|nl|pl|pt-BR|pt-PT|ru|sv|tr|uk|zh-Hans|zh-Hant)\b', text))
        self.assertEqual(documented, bundled)

    def test_local_document_links_resolve(self):
        documents = (
            'README.md', 'README.ja.md', 'PUBLICATION.md',
            'docs/installation.md', 'docs/audio.md',
            'docs/authentication.md', 'docs/updates.md',
            'docs/integrations.md', 'docs/development.md',
            'crates/melba/PACKAGING.md', 'crates/melba/CONTRIBUTING.md',
        )
        for name in documents:
            for link in re.findall(r'\]\(([^)]+)\)', read(name)):
                target = link.split('#', 1)[0]
                if not target or target.startswith(('https://', 'http://', 'mailto:')):
                    continue
                self.assertTrue(
                    ((ROOT / name).parent / target).exists(),
                    f'{name}: broken local link to {link}',
                )

    def test_wrapper_docs_have_both_languages(self):
        self.assertIn('[日本語](wrapper.ja.md)', (ROOT / 'docs/wrapper.md').read_text())
        self.assertIn('[English](wrapper.md)', (ROOT / 'docs/wrapper.ja.md').read_text())

    def test_readme_screenshot_is_referenced_and_present(self):
        image = ROOT / 'docs/assets/melba-album.png'
        self.assertTrue(image.is_file())
        self.assertEqual(image.read_bytes()[:8], b'\x89PNG\r\n\x1a\n')
        self.assertIn('docs/assets/melba-album.png', read('README.md'))
        self.assertIn('docs/assets/melba-album.png', read('README.ja.md'))

    def test_internal_roadmap_is_not_the_front_door(self):
        self.assertNotIn('## Roadmap / TODO', read('README.md'))
        self.assertTrue((ROOT / 'docs/roadmap.md').is_file())


if __name__ == '__main__':
    unittest.main()
