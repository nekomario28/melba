# Custom themes

Melba loads palette files from `~/.config/melba/themes/`. Drop a `*.json`
file there, then pick it under Settings → Appearance (or run
`melba reload-themes`). Up to 128 palettes are listed; keep the folder
under 512 entries.

A palette file names a `base` to start from and any colours it overrides:

```json
{
  "base": "dark",
  "colors": {
    "accent": "#fa2d48",
    "window": "#1c1c1e"
  }
}
```

- `base` is `"dark"` (the default) or `"light"`. Colours you do not set
  come from that base palette.
- Colour values are `"#RRGGBB"` or `"#RRGGBBAA"` (alpha not premultiplied).
- Unknown colour names or malformed values make the file invalid; it is
  skipped and the reason goes to the log
  (`~/.local/state/melba/melba.log`).

## Colour names

| Name | Used for |
|---|---|
| `window` | Main window background |
| `panel` | Side panels and drawers |
| `surface` | Cards, rows, and raised areas |
| `surface_hover` | `surface` under the pointer |
| `surface_active` | Pressed or selected surfaces |
| `outline` | Borders and separators |
| `text` | Primary text |
| `secondary` | Secondary text |
| `dim` | Placeholder and disabled text |
| `accent` | The Apple Music red accent |
| `accent_hover` | Accent under the pointer |
| `on_accent` | Text and icons drawn on the accent |
| `danger` | Destructive actions and errors |
| `warning` | Warnings |
| `overlay` | Dimmed backdrop behind dialogs |
| `shadow` | Shadows under floating elements |
| `tint` | Artwork-driven tint highlights |

The file name becomes the theme name: `Nord.json` shows up as "Nord".
On Omarchy, the live desktop palette is listed as "Omarchy" instead of a
file.
