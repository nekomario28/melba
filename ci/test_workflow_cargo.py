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
