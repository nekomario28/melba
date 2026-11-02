#!/usr/bin/env python3
"""Keep the published GNU ABI guard strict and numerically ordered."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("glibc_abi", ROOT / "ci/check_glibc_abi.py")
abi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(abi)


class GlibcBaselineTest(unittest.TestCase):
    def test_numeric_symbol_comparison(self):
        text = "Name: GLIBC_2.9 (1) Name: GLIBC_2.35 (2) Name: GLIBC_2.3.4 (3)"
        self.assertEqual(abi.required_glibc(text), (2, 35, 0))

    def test_unversioned_output_is_not_treated_as_portable(self):
        with self.assertRaises(ValueError):
            abi.required_glibc("No version information found in this file.")

    @unittest.skipUnless(shutil.which("cc") and shutil.which("readelf"), "native ELF tooling needed")
    def test_actual_elf_and_baseline_boundary(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "smoke.c"
            executable = Path(folder) / "smoke"
            source.write_text('#include <stdio.h>\nint main(void){puts("test");return 0;}\n')
            subprocess.run(["cc", str(source), "-o", str(executable)], check=True, capture_output=True)
            self.assertEqual(abi.main(["99.99", str(executable)]), 0)
            self.assertEqual(abi.main(["2.1", str(executable)]), 1)


if __name__ == "__main__":
    unittest.main()
