#!/usr/bin/env python3
"""Render documentation illustrations from invented copy and geometric art only.

Requires Pillow and an explicitly selected font with Japanese coverage. No app,
network, account, original screenshot or third-party image is opened. These are
illustrations, not screenshots or evidence of playback/backend behaviour.
"""
import argparse
import hashlib
import json
from pathlib import Path

NOTICE = "SYNTHETIC ILLUSTRATION / NOT AN APP CAPTURE"
SCENES = [
    ("ui-album-cjk.png", "album", "ja", 0),
    ("ui-album-en.png", "album", "en", 1),
    ("ui-album-wayland-wgpu.png", "album", "en", 2),
    ("ui-artist-albums-ja.png", "artist-albums", "ja", 3),
    ("ui-artist-en.png", "artist", "en", 4),
    ("ui-artist-ja.png", "artist", "ja", 5),
    ("ui-deeplink-album-en.png", "album", "en", 6),
    ("ui-deeplink-playlist-ja.png", "playlist", "ja", 7),
    ("ui-lyrics-ja.png", "lyrics", "ja", 8),
    ("ui-playlist-en.png", "playlist", "en", 9),
    ("ui-playlist-ja.png", "playlist", "ja", 10),
    ("ui-search-cards-en.png", "cards", "en", 11),
    ("ui-search-scroll-760-ja.png", "search", "ja", 12),
    ("ui-search-scroll-840-ja.png", "search", "ja", 13),
    ("ui-search-sections-en.png", "search", "en", 14),
    ("ui-search-sections-ja.png", "search", "ja", 15),
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def render(scene, font_path):
    from PIL import Image, ImageDraw, ImageFont
    name, page, locale, seed = scene
    w, h = (900, 560) if page == "lyrics" else (720, 480)
    image = Image.new("RGB", (w, h), "#111b28")
    draw = ImageDraw.Draw(image)
    fonts = {n: ImageFont.truetype(str(font_path), n) for n in (11, 13, 15, 20, 24)}
    jp = locale == "ja"
    fg, muted = "#f0f4fa", "#b4c2d4"
    palette = ["#36c2b4", "#f6b875", "#ad98e8", "#e68f9c", "#70b5e1"]

    def text(x, y, value, size=15, color=fg, width=None):
        font = fonts[size]
        if width is not None:
            while value and draw.textlength(value, font=font) > width:
                value = value[:-2] + "…" if len(value) > 2 else ""
        draw.text((x, y), value, font=font, fill=color)

    def art(x, y, size, index):
        # Asymmetric geometric marks, not a photo or a traced album cover.
        a, b = palette[index % len(palette)], palette[(index + 2) % len(palette)]
        draw.rectangle((x, y, x + size, y + size), fill=a)
        draw.ellipse((x + size * .16, y + size * .1, x + size * .78, y + size * .72), fill=b)
        draw.polygon([(x + size * .12, y + size * .88), (x + size * .9, y + size * .28),
                      (x + size * .9, y + size * .88)], fill="#203044")
        draw.line((x + size * .18, y + size * .18, x + size * .7, y + size * .18), fill="#ffffff", width=2)

    # The notice is part of the pixels, not just file metadata.
    draw.rectangle((0, 0, w, 36), fill="#f3d685")
    text(14, 7, NOTICE, 15, "#251d0d")
    draw.rectangle((0, 37, 128, h - 61), fill="#192638")
    text(16, 55, "Melba", 24)
    text(16, 90, "Documentation", 11, muted)
    labels = ["ホーム", "検索", "ライブラリ"] if jp else ["Home", "Search", "Library"]
    for i, label in enumerate(labels):
        text(16, 135 + i * 36, label, 15)
    text(16, 275, "OFFLINE EXAMPLE", 11, palette[0])
    title = {"album": "架空のアルバム" if jp else "Example album",
             "artist": "架空のアーティスト" if jp else "Example artist",
             "artist-albums": "架空のアーティスト" if jp else "Example artist albums",
             "playlist": "架空のプレイリスト" if jp else "Example playlist",
             "cards": "検索結果の見本" if jp else "Example search cards",
             "search": "検索結果の見本" if jp else "Example search results",
             "lyrics": "歌詞レイアウトの見本" if jp else "Example lyric layout"}[page]
    text(150, 54, title, 24, width=w - 172)
    text(150, 89, "Invented data · original geometric artwork · no service connection", 11, muted)

    def track_rows(y, first=0, count=5):
        for i in range(count):
            row_y = y + i * 29
            if row_y + 23 > h - 65:
                break
            if i == 1:
                draw.rounded_rectangle((145, row_y - 2, w - 20, row_y + 24), 4, fill="#294158")
            text(155, row_y, f"{i + first + 1:02d}", 13, muted)
            label = f"合成トラック {i + first + 1:02d}" if jp else f"Example track {i + first + 1:02d}"
            text(193, row_y, label, 15, width=w - 390)
            text(w - 175, row_y, "架空の演奏者" if jp else "Example artist", 11, muted, 115)
            text(w - 55, row_y, "3:00", 13, muted)

    if page in ("album", "artist", "artist-albums", "playlist"):
        art(150, 120, 112, seed)
        text(282, 126, "色と形の練習" if jp else "Studies in colour", 20, width=w - 305)
        text(282, 161, "架空の演奏者 A" if jp else "Example Ensemble A", 15, muted)
        text(282, 191, "Illustrative data only", 13, palette[seed % 5])
        if page == "artist-albums":
            text(150, 250, "アルバム", 15)
            for i in range(3):
                x = 150 + i * 174
                art(x, 282, 68, seed + i)
                text(x, 362, f"合成アルバム {i + 1:02d}", 15, width=156)
                text(x, 388, "架空の演奏者 A", 11, muted)
        else:
            text(150, 250, "トラック" if jp else "Tracks", 13, muted)
            track_rows(278)
    elif page == "cards":
        gap = 16
        card_w = (w - 170 - gap * 2) // 3
        for i in range(3):
            x = 150 + i * (card_w + gap)
            art(x, 132, card_w - 12, seed + i)
            text(x, 302, f"Example album {i + 1}", 15, width=card_w)
            text(x, 328, "Example Ensemble", 11, muted, card_w)
    elif page == "search":
        text(150, 125, "トラック" if jp else "Tracks", 15)
        track_rows(159, first=(seed % 3) * 4, count=5)
        text(150, 324, "アルバム" if jp else "Albums", 15)
        for i in range(3):
            x = 150 + i * 174
            art(x, 354, 42, seed + i)
            text(x + 53, 355, f"Example {i + 1}", 13, width=106)
            text(x + 53, 376, "Invented data", 11, muted, 106)
    else:
        # Newly authored layout-test sentences, not lyrics from a recording.
        lines = ["表示テストの行 01", "これは合成テキストです", "音源に結びつかない文章です",
                 "文字の並びを確認します", "余白と行間の見本です", "表示テストの行 06"]
        for i, line in enumerate(lines):
            text(160, 141 + i * 45, line, 24 if i == 2 else 20,
                 palette[0] if i == 2 else muted, w - 188)
        text(160, 429, "Original test sentences, not service-provided lyrics", 13, muted)

    draw.rectangle((0, h - 60, w, h), fill="#203044")
    art(16, h - 49, 35, seed)
    text(65, h - 48, "合成トラック" if jp else "Example track", 13)
    text(65, h - 26, "No audio output / illustration only", 11, muted)
    text(w - 242, h - 41, "NO ACCOUNT · NO PLAYBACK", 13, palette[0])
    return image


def main():
    import PIL
    from PIL.PngImagePlugin import PngInfo
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", required=True, type=Path, help="Font file with Japanese glyphs")
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    font_bytes = args.font.read_bytes()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {"kind": "synthetic-illustration", "notice": NOTICE,
                "generator_sha256": digest(Path(__file__).read_bytes()),
                "font_sha256": digest(font_bytes), "font_file": args.font.name,
                "pillow_version": PIL.__version__, "images": {}}
    for scene in SCENES:
        image = render(scene, args.font)
        metadata = PngInfo()
        metadata.add_text("MelbaMaterialKind", manifest["kind"])
        metadata.add_text("Description", NOTICE)
        dest = args.output / scene[0]
        image.save(dest, format="PNG", pnginfo=metadata, optimize=False)
        manifest["images"][scene[0]] = {"sha256": digest(dest.read_bytes()),
            "width": image.width, "height": image.height, "page": scene[1], "language": scene[2]}
    (args.output / "synthetic-illustrations.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Rendered {len(SCENES)} synthetic illustrations; no application or service executed.")


if __name__ == "__main__":
    main()
