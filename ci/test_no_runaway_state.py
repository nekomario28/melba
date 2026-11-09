"""Nothing here may append to an unbounded diagnostic file beside user state.

`queue-writes.jsonl` did: one line per queue write, and a queue whose tracks all
fail to open advances continuously. One machine reached 14 GB and 62 million
lines of it before anyone looked.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = [ROOT / "src", ROOT / "crates"]

# Files that must not be written beside user state again.
BANNED = ["queue-writes.jsonl"]


class NoRunawayState(unittest.TestCase):
    def test_the_banned_files_are_not_written(self):
        found = []
        for root in SOURCES:
            for path in root.rglob("*.rs"):
                if "target" in path.parts:
                    continue
                text = path.read_text(encoding="utf-8", errors="replace")
                for name in BANNED:
                    if name in text:
                        found.append(f"{path.relative_to(ROOT)}: {name}")
        self.assertEqual(found, [], "an unbounded diagnostic file came back")

    def test_queue_persistence_is_rate_limited(self):
        """The write itself has to be bounded, not just the recorder.

        A failing queue advances in a loop; without this, the state file is
        rewritten as fast as the loop turns.
        """
        player = (ROOT / "src" / "player.rs").read_text(encoding="utf-8")
        self.assertIn("PERSIST_INTERVAL", player)
        self.assertRegex(
            player,
            r"fn persist_due\(",
            "the throttle decision must be one testable function",
        )
        # Every write goes through the throttled path; a bare `persist_queue`
        # inside the loop would reintroduce the storm.
        self.assertIn("fn persist_queue_with(", player)
        # And the loop itself must give up instead of walking a queue that
        # cannot play: the throttle bounds the writes, this bounds the spinning.
        self.assertIn("failed_opens", player)
        self.assertRegex(
            player,
            r"if failed_opens >= queued",
            "an unplayable queue must stop with a reason",
        )


if __name__ == "__main__":
    unittest.main()


class GateScriptIsolation(unittest.TestCase):
    """`ci/check.sh` starts a real daemon; it must not start it on live state."""

    def test_the_gate_script_isolates_xdg(self):
        text = (ROOT / "ci" / "check.sh").read_text(encoding="utf-8")
        self.assertIn("XDG_STATE_HOME", text, "the gates would write the live queue")
        for var in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME"):
            self.assertIn(var, text, f"{var} is not isolated either")
        # The export must come before anything spawns a binary.
        self.assertLess(
            text.index("XDG_STATE_HOME"),
            text.index("$BIN serve"),
            "isolation has to be in place before the daemon starts",
        )
