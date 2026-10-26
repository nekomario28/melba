#!/usr/bin/env python3
"""Guard against successful scans with missing copyrights or stale staged notices."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("license_bundle", ROOT / "packaging/licenses.py")
licenses = importlib.util.module_from_spec(spec)
spec.loader.exec_module(licenses)


class NoticeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="melba-notices-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.package = self.root / "dependency"
        self.package.mkdir()
        self.identifier = "registry+https://github.com/rust-lang/crates.io-index#fixture@1.0.0"
        self.metadata = {"id": self.identifier, "name": "fixture", "version": "1.0.0",
                         "source": "registry+https://github.com/rust-lang/crates.io-index",
                         "manifest_path": str(self.package / "Cargo.toml")}
        self.report = {"crates": [{"package": self.metadata}], "licenses": [
            {"id": "MIT", "text": "generic MIT text without a copyright owner",
             "source_path": None, "used_by": [{"crate": self.metadata}]}]}
        self.supplements = {"upstream": [], "extra_files": []}

    def render(self):
        return licenses.render(self.report, self.supplements, self.root)

    def test_generic_fallback_is_not_an_original_notice(self):
        # cargo-about --fail can succeed with a synthesized MIT template.
        with self.assertRaisesRegex(ValueError, "no original notice"):
            self.render()

    def test_registry_package_cannot_borrow_a_neighbours_copyright(self):
        (self.root / "LICENSE").write_text("Copyright wrong author")
        with self.assertRaisesRegex(ValueError, "no original notice"):
            self.render()

    def test_git_workspace_copyright_and_nested_notice_are_kept(self):
        (self.root / ".git").mkdir()
        (self.root / "LICENSE-MIT").write_text("Copyright the workspace authors — 原文")
        (self.package / "vendor").mkdir()
        (self.package / "vendor/NOTICE").write_text("Additional original attribution")
        self.metadata["source"] = "git+https://example.invalid/repo#012345"
        text = self.render()
        self.assertIn("Copyright the workspace authors — 原文", text)
        self.assertIn("Additional original attribution", text)
        self.assertNotIn(str(self.root), text)

    def test_local_workspace_notices_do_not_depend_on_git_metadata(self):
        # Source archives must retain the same shared notices as Git checkouts.
        (self.package / "LICENSE").write_text("Copyright subcrate author")
        (self.root / "LICENSE-MIT").write_text("Copyright workspace authors")
        (self.root / "NOTICE.md").write_text("Shared workspace attribution")
        self.metadata["source"] = None
        results = []
        with patch.object(licenses, "ROOT", self.root):
            for has_git in (False, True):
                if has_git:
                    (self.root / ".git").mkdir()
                text = self.render()
                self.assertIn("Copyright subcrate author", text)
                self.assertIn("Copyright workspace authors", text)
                self.assertIn("Shared workspace attribution", text)
                results.append(text)
        self.assertEqual(*results)

    def test_local_package_ids_do_not_leak_build_machine_paths(self):
        self.metadata["source"] = None
        self.metadata["id"] = f"path+file://{self.package}#fixture@1.0.0"
        (self.package / "LICENSE").write_text("Copyright local author")
        text = self.render()
        self.assertNotIn(str(self.root), text)
        self.assertIn("fixture@1.0.0 (local project)", text)

    def test_readme_license_identified_by_the_scanner_is_preserved(self):
        path = self.package / "README.md"
        path.write_text("Copyright the author\nPermission notice in README\n")
        self.report["licenses"][0]["source_path"] = str(path)
        self.assertIn(path.read_text(), self.render())

    def test_empty_graph_and_uncovered_dependency_fail_closed(self):
        self.report["crates"] = []
        with self.assertRaisesRegex(ValueError, "empty"):
            self.render()
        self.report["crates"] = [{"package": self.metadata}]
        self.report["licenses"][0]["used_by"] = [{"crate": {"id": "different-package"}}]
        with self.assertRaisesRegex(ValueError, "incomplete license coverage"):
            self.render()

    def test_original_text_is_required_even_with_a_valid_supplement_hash(self):
        path = self.root / "original.txt"
        path.write_bytes(b"")
        self.supplements["upstream"].append({"package": self.identifier, "path": path.name,
            "url": "https://example.invalid/pinned/LICENSE", "sha256": licenses.digest(b"")})
        with self.assertRaisesRegex(ValueError, "no original notice"):
            self.render()

    def test_pinned_supplement_retains_author_and_is_version_specific(self):
        original = b"Copyright original author\nFull original permission text\n"
        (self.root / "original.txt").write_bytes(original)
        self.supplements["upstream"].append({"package": self.identifier, "path": "original.txt",
            "url": "https://example.invalid/immutable-commit/LICENSE", "sha256": licenses.digest(original)})
        self.assertIn(original.decode(), self.render())
        self.metadata["id"] = self.identifier.replace("1.0.0", "2.0.0")
        with self.assertRaisesRegex(ValueError, "no original notice"):
            self.render()

    def test_corrupted_supplement_is_rejected(self):
        (self.root / "original.txt").write_text("Tampered original text")
        self.supplements["upstream"].append({"package": self.identifier, "path": "original.txt",
            "url": "https://example.invalid/pinned/LICENSE", "sha256": "0" * 64})
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.render()

    def test_nonstandard_font_notice_is_required_and_hash_bound(self):
        (self.package / "LICENSE").write_text("Copyright crate author")
        self.supplements["extra_files"].append({"package": self.identifier,
            "path": "fonts/Hack-Regular.txt", "sha256": "0" * 64})
        with self.assertRaises(FileNotFoundError):
            self.render()
        path = self.package / "fonts/Hack-Regular.txt"
        path.parent.mkdir()
        path.write_bytes(b"Bitstream Vera font copyright and permission")
        with self.assertRaisesRegex(ValueError, "embedded asset notice changed"):
            self.render()
        self.supplements["extra_files"][0]["sha256"] = licenses.digest(path.read_bytes())
        self.assertIn(path.read_text(), self.render())

    def test_mpl_source_availability_has_exact_version_url(self):
        (self.package / "LICENSE").write_text("Mozilla Public License text")
        self.report["licenses"][0]["id"] = "MPL-2.0"
        self.assertIn("https://crates.io/api/v1/crates/fixture/1.0.0/download", self.render())

    def stage(self):
        bundles = {}
        for name in licenses.BUNDLES:
            content = (name + " original notice").encode()
            (self.root / name).write_bytes(content)
            bundles[name] = licenses.digest(content)
        receipt = {"schema": 1, "inputs": licenses.inputs(), "bundles": bundles}
        (self.root / licenses.RECEIPT).write_text(json.dumps(receipt))
        return receipt

    def test_package_check_detects_missing_and_tampered_bundles(self):
        self.stage()
        licenses.check(self.root)
        path = self.root / licenses.BUNDLES[0]
        path.write_text("corruption")
        with self.assertRaisesRegex(ValueError, "bundle checksum mismatch"):
            licenses.check(self.root)
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            licenses.check(self.root)

    def test_package_check_detects_stale_inputs_and_incomplete_bundle_set(self):
        receipt = self.stage()
        receipt["inputs"]["Cargo.lock"] = "stale"
        (self.root / licenses.RECEIPT).write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, "stale notices"):
            licenses.check(self.root)
        receipt = self.stage()
        receipt["bundles"].pop(licenses.BUNDLES[0])
        (self.root / licenses.RECEIPT).write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, "incomplete notice bundle set"):
            licenses.check(self.root)


class ShellGateTests(unittest.TestCase):
    def test_shell_gate_propagates_failure_from_either_cargo_graph(self):
        # Exercise the public shell entry point, not just the Python exception path.
        with tempfile.TemporaryDirectory(prefix="melba-license-shell-") as directory:
            work = Path(directory)
            fakebin = work / "bin"
            fakebin.mkdir()
            (work / "LICENSE").write_text("Copyright synthetic fixture author\nMIT permission fixture\n")
            scanner = fakebin / "cargo-about"
            scanner.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
if "--version" in sys.argv:
    print("cargo-about fixture"); sys.exit(0)
manifest = sys.argv[sys.argv.index("--manifest-path") + 1]
graph = "melba" if manifest.endswith("crates/melba/Cargo.toml") else "melbad"
with open(os.environ["SCAN_TRACE"], "a") as trace:
    trace.write(graph + "\\n")
if graph == os.environ["FAIL_GRAPH"]:
    print("fixture: rejected " + graph + " graph", file=sys.stderr); sys.exit(23)
directory = pathlib.Path(os.environ["FIXTURE_DIR"])
package = {"id": "registry+fixture#fixture@1.0.0", "name": "fixture", "version": "1.0.0",
           "source": "registry+fixture", "manifest_path": str(directory / "Cargo.toml")}
print(json.dumps({"crates": [{"package": package}], "licenses": [{"id": "MIT",
    "text": "MIT fixture", "source_path": str(directory / "LICENSE"),
    "used_by": [{"crate": package}]}]}))
''')
            scanner.chmod(0o755)
            for graph in ("melbad", "melba", ""):
                with self.subTest(graph=graph):
                    trace = work / f"{graph}.trace"
                    output = work / f"{graph}-notices"
                    result = subprocess.run(
                        ["bash", str(ROOT / "ci/licenses.sh"), str(output)],
                        cwd=work, env={**os.environ, "PATH": f"{fakebin}:{os.environ['PATH']}",
                            "FAIL_GRAPH": graph, "SCAN_TRACE": str(trace), "FIXTURE_DIR": str(work)},
                        text=True, capture_output=True, timeout=20,
                    )
                    self.assertEqual(trace.read_text().splitlines(),
                                     ["melbad"] if graph == "melbad" else ["melbad", "melba"])
                    if graph:
                        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertIn(f"rejected {graph} graph", result.stderr)
                        self.assertFalse((output / "receipt.json").exists(),
                                         "partial success must not bless a failed second graph")
                    else:
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        licenses.check(output)


@unittest.skipUnless(shutil.which("cargo-about"), "cargo-about integration requires the build-time collector")
class CollectorIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="melba-license-collector-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text("")
        (self.root / "Cargo.toml").write_text(
            '[package]\nname="notice-fixture"\nversion="1.0.0"\nlicense="MIT"\n'
            '[dependencies]\ndep={path="dep"}\n[workspace]\n')
        (self.root / "LICENSE").write_bytes((ROOT / "LICENSE-MIT").read_bytes())
        self.dep = self.root / "dep"
        (self.dep / "src").mkdir(parents=True)
        (self.dep / "src/lib.rs").write_text("")
        self.manifest = '[package]\nname="dep"\nversion="1.0.0"\nlicense="MIT"\n'
        (self.dep / "Cargo.toml").write_text(self.manifest)
        subprocess.run(["cargo", "generate-lockfile", "--offline", "--manifest-path",
                        str(self.root / "Cargo.toml")], check=True, capture_output=True)

    def collect(self):
        return subprocess.run(["cargo-about", "generate", "--frozen", "--fail", "--format", "json",
            "--config", str(licenses.DATA / "about.toml"), "--manifest-path", str(self.root / "Cargo.toml")],
            text=True, capture_output=True)

    def test_real_collector_fallback_is_rejected_then_original_is_preserved(self):
        result = self.collect()
        self.assertEqual(result.returncode, 0, result.stderr)
        # The upstream tool really does succeed without the dependency copyright.
        with self.assertRaisesRegex(ValueError, "no original notice"):
            licenses.render(json.loads(result.stdout), {"upstream": [], "extra_files": []})
        original = (ROOT / "LICENSE-MIT").read_text()
        (self.dep / "LICENSE").write_text(original)
        result = self.collect()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(original, licenses.render(json.loads(result.stdout), {"upstream": [], "extra_files": []}))

    def test_real_collector_selects_permitted_dedication_and_or_alternative(self):
        # Policy selection is the collector's job; original-notice enforcement is separate.
        for expression, chosen in (("CC0-1.0", "CC0-1.0"),
                                   ("Apache-2.0 OR GPL-2.0-only", "Apache-2.0")):
            with self.subTest(expression=expression):
                (self.dep / "Cargo.toml").write_text(
                    self.manifest.replace('"MIT"', json.dumps(expression)))
                result = self.collect()
                self.assertEqual(result.returncode, 0, result.stderr)
                report = json.loads(result.stdout)
                selected = {(license["id"], use["crate"]["name"])
                            for license in report["licenses"] for use in license["used_by"]}
                self.assertIn((chosen, "dep"), selected)
                self.assertNotIn(("GPL-2.0-only", "dep"), selected)

    def test_real_collector_rejects_disallowed_license(self):
        (self.dep / "Cargo.toml").write_text(self.manifest.replace('"MIT"', '"GPL-3.0-only"'))
        result = self.collect()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("GPL-3.0-only", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
