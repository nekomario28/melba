"""Cargo in CI must resolve from the lockfile, and the audit must be a gate.

A workflow step without `--locked` is allowed to update the lockfile while it
runs, so the job can pass on a dependency graph nobody reviewed — and the next
person's local `--locked` build fails on a lockfile CI had already changed.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"

# Subcommands that take `--locked`. `cargo fmt` does not.
LOCKABLE = re.compile(r"\bcargo (build|check|clippy|test|run|fetch|tree|bench)\b([^\n|&;]*)")


class LockedBuilds(unittest.TestCase):
    def test_every_cargo_invocation_in_ci_is_locked(self):
        unlocked = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                stripped = line.strip()
                if stripped.startswith("#") or stripped.startswith("echo "):
                    continue
                for match in LOCKABLE.finditer(line):
                    if "--locked" not in match.group(2):
                        unlocked.append(f"{path.name}:{number}: {match.group(0).strip()}")
        self.assertEqual(unlocked, [], "these let cargo resolve past the lockfile")

    def test_pinned_tools_are_fetched_through_the_helper(self):
        """A release-asset outage must not fail unrelated pull requests.

        github.com/.../releases/download returned 504 for over an hour while the
        API served the same bytes, which failed gitleaks, cargo-audit and every
        installer smoke. The helper tries the direct route and falls back to the
        API, and it verifies the pinned sha256 either way.
        """
        ci = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        helper = (ROOT / "ci" / "fetch-release-asset.sh").read_text(encoding="utf-8")
        bare = [
            line.strip()
            for line in ci.splitlines()
            if "releases/download/" in line and "fetch-release-asset.sh" not in line
        ]
        self.assertEqual(bare, [], "these download assets without the fallback")
        self.assertIn("api.github.com", helper, "no API fallback")
        self.assertIn("sha256sum -c", helper, "the download is not verified")
        # Both pinned tools must go through it.
        self.assertGreaterEqual(ci.count("ci/fetch-release-asset.sh"), 2)

    def test_the_installer_can_reach_release_assets_when_github_504s(self):
        text = (ROOT / "packaging" / "install-release.sh").read_text(encoding="utf-8")
        self.assertIn("api.github.com/repos/nekomario28/melba/releases", text)
        self.assertIn("application/octet-stream", text, "the API asset route is unused")

    def test_a_mirror_outage_is_reported_not_silently_passed(self):
        """The published-installer job must not claim a distro was verified.

        fedora-43 failed twice in a row on its own mirror while the release
        published fine. Retrying is right; passing quietly is not, so the skip
        has to be a warning with the reason in it.
        """
        text = (WORKFLOWS / "release-publish.yml").read_text(encoding="utf-8")
        self.assertIn("All mirrors were tried", text, "mirror failures are not classified")
        self.assertIn("::warning", text, "a skipped verification must be visible")
        self.assertIn("attempt", text, "a transient mirror failure should be retried")

    def test_public_release_downloads_are_isolated_from_routine_ci(self):
        """Public Release download counts must not grow for ordinary PRs.

        PR candidate smokes use Actions artifacts, while actual signed Release
        installs remain a release-time, installer-change or periodic check.
        """
        ci = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        baseline = (WORKFLOWS / "portable-baseline.yml").read_text(encoding="utf-8")
        distro = (WORKFLOWS / "distro-install.yml").read_text(encoding="utf-8")
        nix = (WORKFLOWS / "nix-paired.yml").read_text(encoding="utf-8")
        publish = (WORKFLOWS / "release-publish.yml").read_text(encoding="utf-8")

        self.assertNotIn("sh packaging/install-release.sh --no-alac", ci)
        self.assertNotIn("nix build --no-link ./crates/melba#melba-paired", ci)
        self.assertIn("actions/download-artifact@", baseline)
        self.assertIn("candidate smoke", baseline)
        self.assertIn("schedule:", distro)
        self.assertIn("pull_request:", distro)
        self.assertNotIn('"docs/installation.md"', distro)
        self.assertIn("schedule:", nix)
        self.assertIn("pull_request:", nix)
        self.assertIn('"crates/melba/nix/**"', nix)
        self.assertIn("nix build --no-link ./crates/melba#melba-paired", nix)
        self.assertIn("published-release-smoke:", publish)
        self.assertIn('gh release download "$TAG"', publish)

    def test_cargo_audit_fails_the_build(self):
        """The exemption existed for an advisory with no fixed release.

        rustls-webpki 0.103.16 carries the fix, so the reason is gone. Keeping
        the job non-blocking after that would mean the advisories it reads are
        never acted on.
        """
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        found = re.search(r"^  cargo-audit:\n((?:    [^\n]*\n)+)", text, re.MULTILINE)
        self.assertIsNotNone(found, "cargo-audit job is missing")
        self.assertNotIn(
            "continue-on-error",
            found.group(1),
            "cargo-audit must fail the build",
        )


if __name__ == "__main__":
    unittest.main()
