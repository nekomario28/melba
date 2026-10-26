#!/usr/bin/env python3
"""Exercise packaging hooks; the native-LTO guard links a tiny C/Rust fixture."""
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
PKGBUILD = ROOT / "packaging/PKGBUILD"


def notice_tool_stub(work, fakebin, env):
    """Stub the external scanner only; run the real rendering and receipt checks."""
    dependency = work / "dependency"
    dependency.mkdir()
    (dependency / "LICENSE").write_text("Copyright fixture author — preserve me\nOriginal permission text\n")
    env["NOTICE_FIXTURE_DIR"] = str(dependency)
    tool = fakebin / "cargo-about"
    tool.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
if "--version" in sys.argv:
    print("cargo-about 0.8.4"); sys.exit(0)
if os.environ.get("ABOUT_FAIL"):
    print("fixture: original notice is missing", file=sys.stderr); sys.exit(1)
directory = pathlib.Path(os.environ["NOTICE_FIXTURE_DIR"])
package = {"id": "registry+fixture#dependency@1.0.0", "name": "dependency", "version": "1.0.0",
           "source": "registry+fixture", "manifest_path": str(directory / "Cargo.toml")}
print(json.dumps({"crates": [{"package": package}], "licenses": [{"id": "MIT",
    "text": (directory / "LICENSE").read_text(), "source_path": str(directory / "LICENSE"),
    "used_by": [{"crate": package}]}]}))
''')
    tool.chmod(0o755)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        # Keep native compiler output off the host's memory-backed /tmp.
        cache = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "melba-packaging-tests"
        cache.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(prefix="melba-packaging-", dir=cache)
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.srcdir = self.work / "src"
        self.checkout = self.srcdir / "apple-music-linux"
        self.checkout.mkdir(parents=True)
        self.env = {
            **os.environ,
            "CARCH": "x86_64",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_AUTHOR_NAME": "Packaging test",
            "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "Packaging test",
            "GIT_COMMITTER_EMAIL": "test@example.invalid",
        }
        self.git("init", "-q")
        self.commit()

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.checkout), *args], env=self.env,
            check=True, text=True, capture_output=True,
        ).stdout.strip()

    def commit(self):
        self.git("-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "fixture")

    def hook(self, name, *, pipefail=False):
        # Do not rely on makepkg (or a developer's shell) enabling pipefail.
        return subprocess.run(
            ["bash", "-c", 'set -e; ' + ('set -o pipefail; ' if pipefail else '')
             + 'source "$1"; srcdir=$2; pkgdir=$3; ' + name,
             "bash", str(PKGBUILD), str(self.srcdir), str(self.work / "pkg")],
            env=self.env, text=True, capture_output=True,
        )

    @unittest.skipUnless(
        shutil.which("gcc") and shutil.which("ar") and shutil.which("rustc")
        and Path("/usr/share/makepkg/buildenv/lto.sh").is_file(),
        "native-LTO integration requires GCC, Rust and makepkg's buildenv helpers",
    )
    def test_native_archive_links_with_rust_under_makepkg_lto_defaults(self):
        # Rust's lld needs native machine code, not GCC-LTO-only archives.
        flags = subprocess.run(
            ["bash", "-c", 'set -e; CARCH=x86_64; CFLAGS=-O2; CXXFLAGS=-O2; LDFLAGS=; '
             'LTOFLAGS=-flto; OPTIONS=(lto); options=(); source "$1"; '
             'source /usr/share/makepkg/buildenv/lto.sh; buildenv_lto; printf "%s" "$CFLAGS"',
             "bash", str(PKGBUILD)], check=True, text=True, capture_output=True,
        ).stdout
        (self.work / "native.c").write_text("int native_sentinel(void) { return 42; }\n")
        (self.work / "main.rs").write_text(
            'extern "C" { fn native_sentinel() -> i32; }\n'
            'fn main() { println!("{}", unsafe { native_sentinel() }); }\n')
        subprocess.run(["gcc", *shlex.split(flags), "-c", "native.c", "-o", "native.o"],
                       cwd=self.work, check=True, capture_output=True)
        subprocess.run(["ar", "crs", "libnative.a", "native.o"],
                       cwd=self.work, check=True, capture_output=True)
        linked = subprocess.run(
            ["rustc", "--edition=2021", "-C", "link-arg=-fuse-ld=lld", "main.rs",
             "-L", "native=.", "-l", "static=native", "-o", "roundtrip"],
            cwd=self.work, text=True, capture_output=True,
        )
        self.assertEqual(linked.returncode, 0, linked.stderr)
        result = subprocess.run([str(self.work / "roundtrip")], check=True,
                                text=True, capture_output=True)
        self.assertEqual(result.stdout, "42\n")

    def test_pkgver_without_tags(self):
        # A git-describe pipeline once hid failure and returned an empty version.
        count = self.git("rev-list", "--count", "HEAD")
        sha = self.git("rev-parse", "--short", "HEAD")
        for pipefail in (False, True):
            with self.subTest(pipefail=pipefail):
                result = self.hook("pkgver", pipefail=pipefail)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), f"0.3.1.r{count}.{sha}")

    def test_pkgver_tracks_tags_and_commits(self):
        for tag, annotated in (("v0.3.0", False), ("v0.4.0", True)):
            with self.subTest(annotated=annotated):
                self.git("-c", "tag.gpgsign=false", "tag",
                         *(["-a", "-m", "release"] if annotated else []), tag)
                for count in (0, 1):
                    if count:
                        self.commit()
                    sha = self.git("rev-parse", "--short", "HEAD")
                    result = self.hook("pkgver")
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.strip(), f"{tag[1:]}.{count}.g{sha}")

    def test_package_stages_build_outputs_even_with_custom_cargo_target(self):
        # Stub only Cargo, the external build boundary; run the real hooks/install/sed.
        fakebin = self.work / "bin"
        fakebin.mkdir()
        cargo = fakebin / "cargo"
        cargo.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ["CARGO_TRACE"], "a") as log:
    log.write(json.dumps(args) + "\\n")
if args[0] == "build":
    name = args[args.index("--bin") + 1] if "--bin" in args else "melba"
    triple = args[args.index("--target") + 1] if "--target" in args else os.environ.get("CARGO_BUILD_TARGET", "")
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", "target")) / triple / "release"
    target.mkdir(parents=True, exist_ok=True)
    (target / name).write_text(f"#!/bin/sh\\n# {name}\\nexit 0\\n")
''')
        cargo.chmod(0o755)
        notice_tool_stub(self.work, fakebin, self.env)
        self.env.update(
            PATH=f"{fakebin}:{os.environ['PATH']}",
            CARGO_TARGET_DIR=str(self.work / "external-target"),
            CARGO_BUILD_TARGET="aarch64-unknown-linux-gnu",
            CARGO_TRACE=str(self.work / "cargo.jsonl"),
        )
        license_dir = "usr/share/licenses/melba-git"
        assets = {
            "LICENSE-MIT": f"{license_dir}/LICENSE-MIT",
            "LICENSE-APACHE": f"{license_dir}/LICENSE-APACHE",
            "NOTICE.md": f"{license_dir}/NOTICE.md",
            "crates/melba/LICENSE": f"{license_dir}/melba-LICENSE",
            "crates/melba-cdm/third_party/cdm-api/LICENSE": f"{license_dir}/cdm-api-LICENSE",
            "crates/melba/assets/fonts/Inter-LICENSE.txt": f"{license_dir}/Inter-LICENSE.txt",
            "crates/melba/assets/fonts/NotoEmoji-LICENSE.txt": f"{license_dir}/NotoEmoji-LICENSE.txt",
            "crates/melba/assets/icons/LICENSE.txt": f"{license_dir}/lucide-LICENSE.txt",
            "crates/melba/packaging/melba.desktop": "usr/share/applications/melba.desktop",
            "crates/melba/packaging/icons/melba.svg": "usr/share/icons/hicolor/scalable/apps/melba.svg",
        }
        for name in ["packaging/melbad.service", *assets, "packaging/licenses.py",
                     "Cargo.toml", "Cargo.lock", "crates/melba/Cargo.toml",
                     "crates/melba/Cargo.lock", "crates/melba-cdm/Cargo.toml"]:
            dest = self.checkout / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
        shutil.copytree(ROOT / "packaging/licenses", self.checkout / "packaging/licenses")
        result = self.hook("prepare; build; check; package")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        pkg = self.work / "pkg"
        for name in ("melba", "melbad"):
            binary = pkg / "usr/bin" / name
            self.assertEqual(binary.read_text(), f"#!/bin/sh\n# {name}\nexit 0\n")
            self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
        for source, dest in assets.items():
            self.assertEqual((pkg / dest).read_bytes(), (ROOT / source).read_bytes())
        unit = (pkg / "usr/lib/systemd/user/melbad.service").read_text()
        self.assertIn("\nExecStart=/usr/bin/melbad serve\n", unit)
        self.assertNotIn(str(ROOT), unit)
        self.assertNotIn(".cargo/bin", unit)
        for name in ("melbad-THIRD-PARTY.txt", "melba-THIRD-PARTY.txt", "receipt.json"):
            self.assertTrue((pkg / license_dir / name).is_file(), name)
        text = (pkg / license_dir / "melba-THIRD-PARTY.txt").read_text()
        self.assertIn("Copyright fixture author — preserve me", text)
        self.assertNotIn(str(self.work), text)
        # Repackaging must not bypass missing/corrupted/stale license artifacts.
        notice = self.srcdir / "third-party/melba-THIRD-PARTY.txt"
        lock = self.checkout / "Cargo.lock"
        for fault in ("missing", "corrupt", "stale"):
            with self.subTest(fault=fault):
                path = lock if fault == "stale" else notice
                original = path.read_bytes()
                if fault == "missing":
                    path.unlink()
                else:
                    path.write_bytes(b"changed")
                try:
                    rejected = self.hook("package")
                    self.assertNotEqual(rejected.returncode, 0, rejected.stdout + rejected.stderr)
                    self.assertIn("licenses:", rejected.stderr)
                finally:
                    path.write_bytes(original)
        commands = [json.loads(line) for line in (self.work / "cargo.jsonl").read_text().splitlines()]
        tests = [args for args in commands if args[0] == "test"]
        self.assertTrue(any("melbad" in args for args in tests), commands)
        self.assertTrue(any("crates/melba/Cargo.toml" in args for args in tests), commands)


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="melba-install-")
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.fakebin = self.work / "bin"
        self.fakebin.mkdir()
        self.root = self.work / "cargo home"
        self.env = {
            **os.environ,
            "PATH": f"{self.fakebin}:/usr/bin:/bin",
            "HOME": str(self.work / "home"),
            "CARGO_HOME": str(self.root),
            "CARGO_INSTALL_ROOT": str(self.root),
            "XDG_DATA_HOME": str(self.work / "data"),
            "XDG_CONFIG_HOME": str(self.work / "config"),
            "XDG_BIN_HOME": str(self.work / "bin"),
            "CARGO_TRACE": str(self.work / "cargo.log"),
            "SYSTEMCTL_TRACE": str(self.work / "systemctl.log"),
        }
        notice_tool_stub(self.work, self.fakebin, self.env)
        self.stub("pkg-config", '#!/bin/sh\nexit 0\n')
        self.stub("xdg-open", '#!/bin/sh\nexit 0\n')
        self.stub("systemctl", '#!/bin/sh\nprintf "%s\\n" "$*" >> "$SYSTEMCTL_TRACE"\n')
        self.stub("cargo", '''#!/usr/bin/env python3
import os, pathlib, sys
args = sys.argv[1:]
with open(os.environ["CARGO_TRACE"], "a") as log:
    log.write(" ".join(args) + "\\n")
if args[0] != "install":
    sys.exit(0)
if os.environ.get("CARGO_FAIL_GUI") and "crates/melba" in args:
    print("fixture: GUI compilation failed", file=sys.stderr); sys.exit(1)
root = args[args.index("--root") + 1] if "--root" in args else os.environ["CARGO_INSTALL_ROOT"]
name = "melba" if "crates/melba" in args else "melbad"
binary = pathlib.Path(root) / "bin" / name
binary.parent.mkdir(parents=True, exist_ok=True)
binary.write_text("#!/bin/sh\\nexit 0\\n")
binary.chmod(0o755)
''')

    def stub(self, name, body):
        path = self.fakebin / name
        path.write_text(body)
        path.chmod(0o755)

    def install(self, *args):
        # Existing assertions pin the Cargo development mode; the default
        # portable path has its own test below.
        return subprocess.run(
            ["/bin/sh", str(ROOT / "packaging/install.sh"), "--cargo", *args],
            cwd=self.work, env=self.env, text=True, capture_output=True,
        )

    def install_portable(self, *args):
        return subprocess.run(
            ["/bin/sh", str(ROOT / "packaging/install.sh"), "--portable", *args],
            cwd=self.work, env=self.env, text=True, capture_output=True,
        )

    def test_missing_webkit_fails_before_installing_anything(self):
        # An incomplete host must fail before Cargo spends minutes compiling.
        self.stub("pkg-config", '#!/bin/sh\n[ "$2" != webkit2gtk-4.1 ]\n')
        result = self.install("--no-unit")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("missing system dev packages for: webkit2gtk-4.1", result.stderr)
        self.assertFalse((self.work / "cargo.log").exists())
        self.assertFalse((self.work / "config").exists())
        self.assertFalse((self.work / "data").exists())

    def test_missing_browser_launcher_fails_before_installing_anything(self):
        # Restrict PATH so a host's xdg-utils cannot hide a fresh-machine failure.
        (self.fakebin / "xdg-open").unlink()
        for program in ("dirname", "mkdir", "sed", "install", "python3"):
            (self.fakebin / program).symlink_to(shutil.which(program))
        self.env["PATH"] = str(self.fakebin)
        result = self.install("--no-unit")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("xdg-open not found", result.stderr)
        self.assertFalse((self.work / "cargo.log").exists())
        self.assertFalse((self.work / "config").exists())
        self.assertFalse((self.work / "data").exists())

    def test_failed_notice_collection_does_not_replace_binaries(self):
        binary = self.root / "bin/melba"
        binary.parent.mkdir(parents=True)
        binary.write_text("old binary")
        self.env["ABOUT_FAIL"] = "1"
        result = self.install("--no-unit")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("original notice is missing", result.stderr)
        self.assertEqual(binary.read_text(), "old binary")
        self.assertFalse((self.root / "bin/melbad").exists())
        self.assertTrue(all(line.startswith("fetch ") for line in
                            (self.work / "cargo.log").read_text().splitlines()))
        self.assertFalse((self.work / "data").exists())
        self.assertFalse((self.work / "systemctl.log").exists())

    def test_gui_build_failure_keeps_notices_for_the_successful_daemon_install(self):
        gui = self.root / "bin/melba"
        gui.parent.mkdir(parents=True)
        gui.write_text("old GUI")
        old_notice = self.root / "share/licenses/melba/melba-THIRD-PARTY.txt"
        old_notice.parent.mkdir(parents=True)
        old_notice.write_text("old GUI copyright")
        self.env["CARGO_FAIL_GUI"] = "1"
        result = self.install("--no-unit")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("GUI compilation failed", result.stderr)
        self.assertTrue((self.root / "bin/melbad").is_file())
        notice = self.root / "share/licenses/melba/melbad-THIRD-PARTY.txt"
        self.assertIn("Copyright fixture author — preserve me", notice.read_text())
        self.assertEqual(gui.read_text(), "old GUI")
        self.assertEqual(old_notice.read_text(), "old GUI copyright")
        self.assertFalse((self.work / "systemctl.log").exists())

    def test_install_pairs_binaries_without_starting_an_independent_daemon(self):
        # A stale executable on PATH must never win over the just-installed one.
        self.stub("melbad", '#!/bin/sh\nexit 99\n')
        for _ in range(2):
            result = self.install()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            unit = (self.work / "config/systemd/user/melbad.service").read_text()
            execstart = next(line.removeprefix("ExecStart=") for line in unit.splitlines()
                             if line.startswith("ExecStart="))
            self.assertEqual(shlex.split(execstart), ["/usr/bin/env", str(self.root / "bin/melbad"), "serve"])
            desktop = (self.work / "data/applications/melba.desktop").read_text()
            command = next(line.removeprefix("Exec=") for line in desktop.splitlines() if line.startswith("Exec="))
            self.assertEqual(shlex.split(command), ["/usr/bin/env", str(self.root / "bin/melba"), "%u"])
        calls = (self.work / "systemctl.log").read_text().splitlines()
        self.assertEqual(calls, ["--user daemon-reload"] * 2, "the desktop owns daemon startup; installation must not autoplay or create a login player")

    def test_default_portable_install_writes_marker_and_paths(self):
        portable = self.work / "portable"
        self.env["MELBA_PORTABLE_DIR"] = str(portable)
        result = self.install_portable("--no-unit")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("melba", "melbad", "melba-portable.txt"):
            self.assertTrue((portable / name).exists(), name)
        self.assertEqual((portable / "melba-portable.txt").read_text(), "melba-portable-v1\n")
        desktop = (self.work / "data/applications/melba.desktop").read_text()
        command = next(line.removeprefix("Exec=") for line in desktop.splitlines() if line.startswith("Exec="))
        self.assertEqual(shlex.split(command), ["/usr/bin/env", str(portable / "melba"), "%u"])
        self.assertEqual((self.work / "bin/melba").resolve(), portable / "melba")
        self.assertEqual((self.work / "bin/melbad").resolve(), portable / "melbad")

    def test_invalid_options_fail_before_build(self):
        for args in (("--unknown",), ("--no-unit", "extra")):
            with self.subTest(args=args):
                result = self.install(*args)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertFalse((self.work / "cargo.log").exists())

    @unittest.skipUnless(shutil.which("gio"), "gio (libglib2.0-bin) is required")
    def test_desktop_launch_handles_special_install_paths(self):
        # Feed the generated Exec line to its real consumer, not a shell approximation.
        self.root = self.work / 'cargo space % $dollar "quote" \\ slash & | `tick`'
        marker = self.work / "launched"
        self.env.update(CARGO_HOME=str(self.root), CARGO_INSTALL_ROOT=str(self.root),
                        LAUNCH_MARKER=str(marker))
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (self.root / "bin/melba").write_text('#!/bin/sh\nprintf launched > "$LAUNCH_MARKER"\n')
        desktop = self.work / "data/applications/melba.desktop"
        result = subprocess.run(["gio", "launch", str(desktop)], env=self.env,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        deadline = time.monotonic() + 2
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertEqual(marker.read_text(), "launched")

    def test_no_unit_installs_desktop_without_touching_systemd(self):
        result = self.install("--no-unit")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.root / "bin/melba").is_file())
        self.assertTrue((self.root / "bin/melbad").is_file())
        self.assertTrue((self.work / "data/applications/melba.desktop").is_file())
        for name in ("melbad-THIRD-PARTY.txt", "melba-THIRD-PARTY.txt"):
            text = (self.root / "share/licenses/melba" / name).read_text()
            self.assertIn("Copyright fixture author — preserve me", text)
        self.assertTrue((self.root / "share/licenses/melba/receipt.json").is_file())
        self.assertFalse((self.work / "systemctl.log").exists())
        self.assertFalse((self.work / "config").exists())


class PortableArchiveTests(unittest.TestCase):
    def test_package_portable_always_includes_the_marker(self):
        with tempfile.TemporaryDirectory(prefix="melba-portable-package-") as folder:
            work = Path(folder)
            melba = work / "melba"
            melbad = work / "melbad"
            melba.write_text("#!/bin/sh\nexit 0\n")
            melbad.write_text("#!/bin/sh\nexit 0\n")
            melba.chmod(0o755)
            melbad.chmod(0o755)
            out = work / "out"
            result = subprocess.run(
                ["/bin/sh", str(ROOT / "packaging/package-portable.sh"),
                 str(out), "9.9.9", str(melba), str(melbad), "x86_64-unknown-linux-gnu"],
                text=True, capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            archive = out / "melba-v9.9.9-x86_64-unknown-linux-gnu.tar.gz"
            names = subprocess.run(["tar", "-tzf", str(archive)], check=True,
                                   text=True, capture_output=True).stdout.splitlines()
            self.assertIn("melba-v9.9.9-x86_64-unknown-linux-gnu/melba-portable.txt", names)
            self.assertIn("melbad-v9.9.9-x86_64-unknown-linux-gnu.tar.gz",
                          [p.name for p in out.iterdir()])


class WrapperStagingTests(unittest.TestCase):
    def test_staging_does_not_claim_to_verify_a_running_service(self):
        # Staging used to print live FairPlay/network success without running anything.
        with tempfile.TemporaryDirectory(prefix="melba-wrapper-stage-") as directory:
            work = Path(directory)
            payload = work / "payload"
            lib = payload / "rootfs/system/lib64"
            lib.mkdir(parents=True)
            (lib / "fixture.so").write_text("synthetic library; never loaded")
            (lib / "argv-scrub.so").write_text("synthetic preload; never loaded")
            tzdata = payload / "rootfs/system/usr/share/zoneinfo/tzdata"
            tzdata.parent.mkdir(parents=True)
            tzdata.write_text("synthetic timezone data")
            fakebin = work / "bin"
            fakebin.mkdir()
            marker = work / "unexpected-execution"
            for path in (payload / "wrapper", fakebin / "curl"):
                path.write_text('#!/bin/sh\nprintf invoked > "$EXECUTION_MARKER"\nexit 97\n')
                path.chmod(0o755)
            result = subprocess.run(
                ["bash", str(ROOT / "packaging/install-wrapper.sh"),
                 "--dir", str(payload), "--arch", "x86_64"],
                env={**os.environ, "PATH": f"{fakebin}:/usr/bin:/bin",
                     "HOME": str(work / "home"), "EXECUTION_MARKER": str(marker)},
                text=True, capture_output=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse(marker.exists(), "staging must not execute the payload or fetch an APK")
            self.assertTrue((payload / "run").is_file())
            self.assertIn("not executed", result.stdout)
            self.assertNotIn("verified:", result.stdout)
            self.assertNotIn("real Apple network calls", result.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
