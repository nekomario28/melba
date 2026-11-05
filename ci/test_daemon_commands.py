#!/usr/bin/env python3
"""Every Command the GUI sends must be handled, or be a recorded exception.

The worker receives commands in one match; anything it does not name falls
into `_ => {}` and is dropped without a log line. That is how seven cache
commands, the radio lookup and three desktop actions sat unhandled: the app
kept sending them and nothing ever answered.

This guard reads the two enums from the sources, so it fails when a variant is
added without a decision. An exception is not permission to forget it: it
carries a reason and must name what the user loses while it stands.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BACKEND = ROOT / "crates/melba/src/backend.rs"
WORKER = ROOT / "crates/melba/src/daemon_worker.rs"

# Variants the worker never sees because Backend::send rewrites them first.
REWRITTEN = {"SignIn": "Backend::send turns it into AuthStart"}

# Sent by the app, not handled by the worker yet. Each entry says who loses
# what, so the list cannot quietly grow.
UNHANDLED = {
    "CheckPlaylistCover": "a chosen cover is never checked against the playlist",
    "ChoosePlaylistCover": "the native cover picker never opens",
    "Accent": "no artwork accent colour is published",
    "NetworkStatus": "offline transitions are not reported to the app",
    "LoadPlaylistCache": "a playlist cannot be shown from disk on a cold start",
    "StorePlaylistCache": "a playlist is never saved for the next start",
    "LoadAccountCaches": "the last library and Home are not shown while reloading",
    "StoreLibraryCache": "the library is never saved for the next start",
    "StoreHomeCache": "Home shelves are never saved for the next start",
    "Radio": "a radio page never resolves its songs",
}


def command_variants():
    body = re.search(r"pub enum Command \{(.*?)\n\}", BACKEND.read_text(), re.S).group(1)
    return re.findall(r"(?m)^\s{4}([A-Z][A-Za-z0-9]*)", body)


def handled_variants():
    lines = WORKER.read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if "commands.recv() =>" in line)
    end = next(i for i in range(start, len(lines)) if re.match(r"\s+_ => \{\}", lines[i]))
    return set(re.findall(r"Command::([A-Z][A-Za-z0-9]*)", "\n".join(lines[start:end + 1])))


class CommandCoverage(unittest.TestCase):
    def test_every_command_is_handled_or_recorded(self):
        variants = command_variants()
        self.assertTrue(variants, "the Command enum was not found")
        handled = handled_variants()
        self.assertTrue(handled, "the worker's receive match was not found")
        unknown = [
            v for v in variants
            if v not in handled and v not in REWRITTEN and v not in UNHANDLED
        ]
        self.assertEqual(
            unknown, [],
            "these commands are sent but nothing handles them, and no exception "
            "records what the user loses: " + ", ".join(unknown),
        )

    def test_the_exception_list_has_no_stale_entries(self):
        """A variant that is now handled, or gone, must leave the list."""
        variants = set(command_variants())
        handled = handled_variants()
        stale = [v for v in UNHANDLED if v in handled or v not in variants]
        self.assertEqual(stale, [], f"recorded exceptions that no longer apply: {stale}")

    def test_a_new_unhandled_variant_is_caught(self):
        variants = command_variants() + ["SomethingNew"]
        handled = handled_variants()
        unknown = [
            v for v in variants
            if v not in handled and v not in REWRITTEN and v not in UNHANDLED
        ]
        self.assertEqual(unknown, ["SomethingNew"])


if __name__ == "__main__":
    unittest.main()
