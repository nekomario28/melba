#!/usr/bin/env python3
"""Render the README project map deterministically.

The map is a projection of the real repository/runtime boundary: it names the
GUI, daemon, ALSA path, catalog/CDM, optional wrapper and signed updater. It is
not a screenshot and carries no account data.
"""
from pathlib import Path
from xml.sax.saxutils import escape

W, H = 1200, 650
BG = "#0f1114"
PANEL = "#15181c"
SURFACE = "#1d2127"
OUTLINE = "#2a3038"
TEXT = "#f2f4f6"
SECONDARY = "#a9b1bc"
DIM = "#6e7784"
ACCENT = "#fa243c"
CYAN = "#6ee7f2"
PURPLE = "#c084fc"
AMBER = "#f2b85c"
GREEN = "#8be28b"

out = []
add = out.append


def text(x, y, value, size=18, fill=TEXT, weight=400, anchor="start", family="Inter, ui-sans-serif, sans-serif", spacing=None):
    extra = f' letter-spacing="{spacing}"' if spacing else ""
    add(f'<text x="{x}" y="{y}" fill="{fill}" font-family="{family}" font-size="{size}" font-weight="{weight}" text-anchor="{anchor}"{extra}>{escape(value)}</text>')


def mono(x, y, value, size=13, fill=SECONDARY, anchor="start"):
    text(x, y, value, size=size, fill=fill, weight=500, anchor=anchor,
         family="ui-monospace, SFMono-Regular, Menlo, monospace")


def card(x, y, w, h, title, subtitle, lines, accent, *,
         title_size=22, mono_title=False):
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="{SURFACE}" stroke="{OUTLINE}" stroke-width="1"/>')
    add(f'<rect x="{x}" y="{y}" width="5" height="{h}" rx="2.5" fill="{accent}"/>')
    tx = x + 24
    if mono_title:
        mono(tx, y + 36, title, 18, TEXT)
    else:
        text(tx, y + 36, title, title_size, TEXT, 700)
    text(tx, y + 62, subtitle, 17, SECONDARY)
    for i, line in enumerate(lines):
        mono(tx, y + 90 + i * 24, line, 17, SECONDARY if i else TEXT)


def connector(points, color, label=None, label_at=None, dash=False):
    d = " ".join(("M" if i == 0 else "L") + f" {x} {y}" for i, (x, y) in enumerate(points))
    dash_attr = ' stroke-dasharray="5 7"' if dash else ""
    add(f'<path d="{d}" fill="none" stroke="{color}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"{dash_attr} opacity="0.72" marker-end="url(#arrow-{color[1:]})"/>')
    if label and label_at:
        text(label_at[0], label_at[1], label, 15, DIM, 500, "middle")


add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" aria-labelledby="title desc">')
add(f'<title id="title">Melba project map</title>')
add('<desc id="desc">A system map of Melba: the native GUI and MPRIS daemon, direct ALSA hw: output, MusicKit catalog and Widevine AAC, an optional user-supplied ALAC wrapper, and signed portable releases.</desc>')
add('<defs>')
for color in (ACCENT, CYAN, PURPLE, AMBER, GREEN):
    add(f'<marker id="arrow-{color[1:]}" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="{color}"/></marker>')
add(f'<linearGradient id="wash" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{ACCENT}" stop-opacity="0.14"/><stop offset="0.5" stop-color="{PURPLE}" stop-opacity="0.05"/><stop offset="1" stop-color="{CYAN}" stop-opacity="0.10"/></linearGradient>')
add('<pattern id="grid" width="40" height="40" patternUnits="userSpaceOnUse"><path d="M 40 0 L 0 0 0 40" fill="none" stroke="#ffffff" stroke-opacity="0.035" stroke-width="1"/></pattern>')
add('</defs>')
add(f'<rect width="{W}" height="{H}" rx="28" fill="{BG}"/>')
add(f'<rect width="{W}" height="{H}" rx="28" fill="url(#wash)"/>')
add(f'<rect width="{W}" height="{H}" rx="28" fill="url(#grid)"/>')
add(f'<rect x="1" y="1" width="{W-2}" height="{H-2}" rx="27" fill="none" stroke="{OUTLINE}" stroke-width="1"/>')

# Header
add(f'<circle cx="58" cy="54" r="13" fill="{ACCENT}"/>')
add(f'<path d="M54 46 L67 54 L54 62 Z" fill="#ffffff"/>')
text(84, 62, "MELBA", 34, TEXT, 800, spacing="1.5")
text(84, 86, "native Apple Music for Linux", 18, SECONDARY)
mono(1116, 54, "v0.3.5", 18, TEXT, "end")
mono(1116, 76, "signed · x86_64 / arm64", 15, DIM, "end")

# Left: GUI
card(48, 126, 286, 210, "GUI", "crates/melba · egui", [
    "Home · Search · Library",
    "Queue · Lyrics · MPRIS",
    "16 locales · themes",
], ACCENT)

# Center: daemon
card(404, 126, 292, 210, "melbad", "playback daemon", [
    "ALSA · MPRIS endpoint",
    "MusicKit streams · queue",
    "Widevine AAC · updater",
], CYAN, mono_title=True)

# Right top: catalog
card(788, 126, 364, 120, "MusicKit", "public catalog + subscription", [
    "catalog · previews",
    "AAC-256 via Widevine",
], GREEN)

# Right bottom: ALAC
card(788, 276, 364, 120, "ALAC", "optional user-supplied wrapper", [
    "user-supplied decrypt service",
    "browser / native sign-in",
], PURPLE)

# Bottom: ALSA / output
card(404, 388, 292, 150, "ALSA", "the output path", [
    "hw:X,Y direct open",
    "BITPERFECT receipt",
    "shared default refused",
], AMBER, mono_title=True)

# Bottom left: release/update
card(48, 388, 286, 150, "Signed releases", "portable install + updater", [
    "melba + melbad together",
    "rollback on failed start",
    "Ed25519 checksums",
], ACCENT)

# Bottom right: trust boundary
card(788, 432, 364, 106, "Boundary", "what stays outside", [
    "wrapper/rootfs user-supplied",
    "no keys or account data shipped",
], DIM)

# Connectors: GUI <-> daemon, daemon <-> catalog/Alsa, wrapper -> daemon
connector([(334, 232), (404, 232)], ACCENT, "MPRIS", (368, 218))
connector([(550, 336), (550, 388)], AMBER, "hw:", (562, 366))
connector([(550, 336), (788, 232)], GREEN, "AAC", (667, 286))
connector([(788, 336), (696, 336)], PURPLE, "ALAC", (742, 324))
connector([(190, 388), (190, 336)], ACCENT, "update", (216, 366))
connector([(970, 432), (970, 396)], DIM, "", None, dash=True)

# A quiet "legend" at the very bottom
add(f'<line x1="48" y1="584" x2="1152" y2="584" stroke="{OUTLINE}" stroke-width="1"/>')
for x, color, label in [(48, ACCENT, "GUI / release"), (228, CYAN, "daemon"), (356, GREEN, "MusicKit"), (500, PURPLE, "ALAC"), (620, AMBER, "ALSA"), (716, DIM, "external boundary")]:
    add(f'<circle cx="{x}" cy="608" r="5" fill="{color}"/>')
    text(x + 14, 613, label, 15, SECONDARY, 500)
mono(1152, 613, "bit-perfect is a path property, not a codec label", 14, DIM, "end")

add('</svg>')
import sys
out_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).with_name("project-map.svg")
out_path.write_text("\n".join(out) + "\n")
print("wrote project-map.svg")
