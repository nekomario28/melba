#!/usr/bin/env python3
"""Source-only publication scope: no tracked binary payloads, no wrapper ops in
the public front docs. This is a scope guard, not a legal or secret audit."""
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN_SUFFIXES = {'.so', '.apk', '.xapk', '.zip', '.exe', '.dll', '.dylib',
                      '.o', '.a'}
# A generic data fixture may be named .bin; an executable payload may be named
# anything. Check content, not only the extension.
EXECUTABLE_MAGICS = (b'\x7fELF', b'MZ', b'\xfe\xed\xfa\xce', b'\xfe\xed\xfa\xcf',
                     b'\xce\xfa\xed\xfe', b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe')
FRONT_DOCS = ('README.md', 'README.ja.md', 'PUBLICATION.md', 'NOTICE.md', 'crates/melba/PACKAGING.md')
OPERATIONAL_PHRASES = ('melbad wrapper setup', '--apk', '40020', '10020',
                       '20020', '30020', 'contentKey', 'skd://')


def verify_payloads(root, paths):
    for name in paths:
        parts = Path(name).parts
        if Path(name).suffix.lower() in FORBIDDEN_SUFFIXES or 'rootfs' in parts:
            raise ValueError('BINARY_PAYLOAD_TRACKED')
        path = root / name
        if path.is_symlink() or not path.is_file():
            continue
        with path.open('rb') as handle:
            head = handle.read(4)
        if any(head.startswith(magic) for magic in EXECUTABLE_MAGICS):
            raise ValueError('BINARY_PAYLOAD_TRACKED')


def verify_docs(texts):
    for name in FRONT_DOCS:
        text = texts.get(name)
        if text is None:
            raise ValueError('PUBLICATION_DOC_MISSING')
        for phrase in OPERATIONAL_PHRASES:
            if phrase in text:
                raise ValueError('OPERATIONAL_WRAPPER_TEXT')


def tracked_paths(root):
    try:
        out = subprocess.run(['git', '-C', str(root), 'ls-files', '-z'],
                             capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        # A source archive has no index; scan the tree instead of skipping the
        # guard. Build output is not part of what would be published.
        return [str(path.relative_to(root)) for path in root.rglob('*')
                if path.is_file() and not {'target', '.git'} & set(path.parts)]
    return [p.decode() for p in out.split(b'\0') if p]


class PublicationScope(unittest.TestCase):
    def test_scope_holds_for_the_current_tree(self):
        verify_payloads(ROOT, tracked_paths(ROOT))
        verify_docs({name: (ROOT / name).read_text()
                     for name in FRONT_DOCS if (ROOT / name).is_file()})

    def test_binary_payload_is_rejected(self):
        with self.assertRaisesRegex(ValueError, '^BINARY_PAYLOAD_TRACKED$'):
            verify_payloads(ROOT, ['fixtures/example.so'])
        with tempfile.TemporaryDirectory(prefix='publication-scope-') as folder:
            root = Path(folder)
            (root / 'data.bin').write_bytes(b'\x7fELF not a real program')
            with self.assertRaisesRegex(ValueError, '^BINARY_PAYLOAD_TRACKED$'):
                verify_payloads(root, ['data.bin'])

    def test_wrapper_operations_are_rejected_in_front_docs(self):
        texts = {name: (ROOT / name).read_text() for name in FRONT_DOCS}
        texts['README.md'] += '\nmelbad wrapper setup\n'
        with self.assertRaisesRegex(ValueError, '^OPERATIONAL_WRAPPER_TEXT$'):
            verify_docs(texts)

    def test_missing_scope_document_is_not_clean(self):
        with self.assertRaisesRegex(ValueError, '^PUBLICATION_DOC_MISSING$'):
            verify_docs({'README.md': ''})


if __name__ == '__main__':
    unittest.main()
