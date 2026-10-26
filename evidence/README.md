# Public documentation images

The files listed in [synthetic-illustrations.json](synthetic-illustrations.json)
are **synthetic illustrations, not application screenshots**. Their pixels,
geometric artwork and example sentences come from
[render_synthetic.py](render_synthetic.py), not from account data, catalog
artwork, artist photos, recordings or service-provided lyrics.

The visible banner is intentional. These images illustrate page concepts; they
do not certify the current app's appearance, localization, deep links, renderer,
scroll behaviour, Wayland support, playback, source format or physical output. Historical filenames
are retained for reference compatibility, not as claims about the rendering
backend or viewport. Other files in this directory are outside this replacement
set and must be assessed separately.

Historical notes that mention these filenames are historical observations, not
new acceptance evidence. The original captures are no longer in the current
working tree. **Git history was not rewritten**: replacing a current file does
not remove older artwork or personal paths from reachable commits.

## Regenerate

Use Python 3, Pillow and an explicitly selected font with Japanese glyphs:

```sh
python3 evidence/render_synthetic.py --font "$(fc-match -f '%{file}' :lang=ja)"
python3 ci/publication_assets.py
```

The renderer uses no application, account, network, original screenshot or audio
runtime. It records the generator digest, font digest and Pillow version in the
manifest. Reproducing the same pixels requires the same font and rendering
version. The font software is not embedded in the PNGs; retain its own license
if distributing the font separately. The newly authored artwork and example
copy follow the project's source license.

The current-tree guard checks image identities, dimensions, provenance labels
and personal-path literals. It is not a complete secret scanner, permission
review or publication authorization. Visual acceptance requires full-size
inspection of the complete set, including whether each layout communicates its
named content. Reviewing source code or listing image filenames is not pixel
inspection; an independent visual reviewer must receive the actual images.
