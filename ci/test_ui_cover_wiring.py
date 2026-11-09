"""A page header must take its cover from the one place that decides it.

The playlist header kept a placeholder while the row beside it showed the group
icon, because the decision was made at each call site and the loaded path forgot
it. It is made once now, inside `playlist_hero`; this holds that shape so the
next header path cannot forget it either.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COLLECTION = ROOT / "crates" / "melba" / "src" / "ui" / "collection.rs"


def function_body(text: str, name: str) -> str:
    start = text.index(f"fn {name}")
    depth = 0
    began = False
    out = []
    for line in text[start:].splitlines(keepends=True):
        out.append(line)
        depth += line.count("{") - line.count("}")
        if "{" in line:
            began = True
        if began and depth == 0:
            break
    return "".join(out)


class PlaylistHeaderCover(unittest.TestCase):
    def test_the_header_fills_the_group_art_where_it_is_drawn(self):
        text = COLLECTION.read_text(encoding="utf-8")
        hero = function_body(text, "playlist_hero")
        self.assertIn(
            "hero_group_art(",
            hero,
            "the header no longer asks for the group art",
        )

    def test_only_one_place_asks_for_it(self):
        """A second call site is how the loaded path was missed."""
        text = COLLECTION.read_text(encoding="utf-8")
        calls = len(re.findall(r"^(\s*)images\.mosaic = hero_group_art\(", text, re.MULTILINE))
        self.assertEqual(calls, 1, "the decision belongs in playlist_hero alone")


if __name__ == "__main__":
    unittest.main()
