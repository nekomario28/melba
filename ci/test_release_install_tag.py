"""The installer must be able to go back to a published release.

Every release stays on the Releases page, so a regression is fixed by
reinstalling the previous tag — with the same signature and checksum checks as
an install of the latest. These assertions are about the installer's *shape*:
that the option exists, that it is validated before anything is downloaded, and
that the tag actually reaches the URL it is fetched from.
"""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "packaging" / "install-release.sh"


def text() -> str:
    return SCRIPT.read_text()


class ReleaseInstallTag(unittest.TestCase):

    def test_the_option_is_documented_in_help(self):
        out = subprocess.run(
            ["sh", str(SCRIPT), "--help"], capture_output=True, text=True, check=True
        )
        assert "--version vX.Y.Z" in out.stdout


    def test_an_unversioned_tag_is_refused_before_downloading(self):
        # `--version 0.3.27` would otherwise build a URL from an unchecked string.
        out = subprocess.run(
            ["sh", str(SCRIPT), "--version", "0.3.27"],
            capture_output=True,
            text=True,
        )
        assert out.returncode == 2, out
        assert "expected latest or vX.Y.Z" in out.stderr


    def test_the_tag_selects_the_url_it_is_fetched_from(self):
        source = text()
        assert "tag=${MELBA_VERSION:-latest}" in source
        assert 'releases/download/$tag' in source
        # the manifest, its signature and the archive all carry the tag
        assert 'fetch "$work/checksums.txt" checksums.txt "$tag"' in source
        assert 'fetch "$work/checksums.txt.sig" checksums.txt.sig "$tag"' in source
        # signature verification stays between the download and the archive choice
        order = [
            source.index('fetch "$work/checksums.txt" checksums.txt "$tag"'),
            source.index("openssl pkeyutl -verify"),
            source.index("archive=$(awk"),
        ]
        assert order == sorted(order), "the manifest is verified before it is trusted"


    def test_no_step_declares_with_twice(self):
        """A duplicate `with:` is accepted by YAML readers and rejected by GitHub.

        It happened here: adding `toolchain:` above an existing `components:` left
        two `with:` keys, the workflow failed to start with zero jobs, and the
        required check simply never appeared.
        """
        for name in ("ci.yml", "portable-baseline.yml", "arm64-build.yml"):
            lines = (ROOT / ".github" / "workflows" / name).read_text().splitlines()
            seen: list[int] = []
            for line in lines:
                indent = len(line) - len(line.lstrip())
                if line.lstrip().startswith("- "):
                    # a new list item is a new mapping; the same indent is fine there
                    seen = []
                if line.strip() == "with:":
                    assert indent not in seen, f"{name}: two `with:` blocks at one level"
                    seen = [i for i in seen if i < indent]
                    seen.append(indent)

    def test_the_release_toolchain_is_pinned(self):
        """A rebuilt tag must produce the same binaries, so the toolchain cannot float."""
        pinned = (ROOT / "rust-toolchain.toml").read_text()
        channel = re.search(r'channel\s*=\s*"([0-9.]+)"', pinned)
        assert channel, "rust-toolchain.toml must name an exact version"
        version = channel.group(1)
        for name in ("ci.yml", "portable-baseline.yml", "arm64-build.yml"):
            workflow = (ROOT / ".github" / "workflows" / name).read_text()
            for block in workflow.split("dtolnay/rust-toolchain@")[1:]:
                head = block.split("\n      - ")[0][:600]
                assert f'toolchain: "{version}"' in head, (
                    f"{name} installs a floating toolchain; pin {version}"
                )


if __name__ == "__main__":
    unittest.main()


class ReleaseVersionLock(unittest.TestCase):
    """A version bump has to reach every lock file the build uses.

    `crates/melba` is its own workspace with its own `Cargo.lock`; bumping only the
    outer one left that lock naming 0.3.27, and `cargo fetch --locked` failed in
    CI — on every job that fetches, which read as four unrelated failures.
    """

    def test_every_lock_file_names_the_manifest_version(self):
        root = ROOT
        pairs = [
            (root / "Cargo.toml", root / "Cargo.lock", "melbad"),
            (
                root / "crates" / "melba" / "Cargo.toml",
                root / "crates" / "melba" / "Cargo.lock",
                "melba",
            ),
        ]
        for manifest, lock, package in pairs:
            version = None
            for line in manifest.read_text().splitlines():
                if line.startswith("version = "):
                    version = line.split('"')[1]
                    break
            self.assertIsNotNone(version, f"{manifest}: no version")
            text = lock.read_text()
            marker = f'name = "{package}"\nversion = "{version}"'
            self.assertIn(
                marker,
                text,
                f"{lock.name} does not name {package} {version}; run cargo update "
                f"--manifest-path {manifest}",
            )


class ReleaseChannelPublishing(unittest.TestCase):
    def test_release_version_contract_accepts_stable_and_alpha_only(self):
        script = ROOT / "ci" / "release-version.sh"
        for version, expected in (
            ("0.3.30", "stable"),
            ("0.4.0", "stable"),
            ("0.4.0-alpha.0", "alpha"),
            ("0.4.0-alpha.1", "alpha"),
            ("0.4.0-alpha.12", "alpha"),
        ):
            out = subprocess.run(
                ["bash", str(script), version],
                capture_output=True,
                text=True,
            )
            self.assertEqual(out.returncode, 0, (version, out.stderr))
            self.assertEqual(out.stdout.strip(), expected)

        for bad in (
            "v0.4.0",
            "0.4.0-beta.1",
            "0.4.0-rc.1",
            "0.4.0-alpha",
            "0.4.0-alpha.01",
            "04.0.0",
            "0.4.0+build",
            "0.4.0-alpha.1/../x",
        ):
            out = subprocess.run(
                ["bash", str(script), bad],
                capture_output=True,
                text=True,
            )
            self.assertEqual(out.returncode, 2, (bad, out.stdout, out.stderr))

    def test_prepare_signed_release_delegates_version_check_for_alpha(self):
        env = {k: v for k, v in __import__("os").environ.items() if k != "MELBA_SIGNING_KEY_PEM"}
        out = subprocess.run(
            ["bash", str(ROOT / "ci" / "prepare_signed_release.sh"), "/tmp/does-not-exist", "0.4.0-alpha.1"],
            capture_output=True,
            text=True,
            env=env,
        )
        # Fails on missing key (exit 1), not invalid version (exit 2).
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertIn("MELBA_SIGNING_KEY_PEM is absent", out.stderr)

    def test_release_publish_keeps_alpha_off_latest_and_smokes_the_published_tag(self):
        text = (ROOT / ".github" / "workflows" / "release-publish.yml").read_text(encoding="utf-8")
        self.assertIn("bash ci/release-version.sh", text)
        self.assertIn("--prerelease", text)
        self.assertIn("--latest=false", text)
        self.assertIn('install-release.sh --no-alac --version "$TAG"', text)
