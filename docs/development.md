# Development and verification

[README](../README.md) · [Roadmap](roadmap.md) · [Agent-facing specs](../specs/README.md)

User-facing README text describes practical capabilities; regression cases and evidence are maintained separately to keep test coverage distinct from installed-runtime acceptance.

## UI without a display

The headless tests exercise real egui views with API-shaped fixtures, in a private HOME/XDG hierarchy and session bus, without a display, GPU, live account credentials or audio playback.

```sh
cargo test --locked --manifest-path crates/melba/Cargo.toml --all-features --lib app::headless_tests
```

The image contract requires a successful fetch **and decode** followed by a nontransparent, clipped drawing mesh; a valid URL alone is insufficient. Selecting a non-first album row through accessibility or pointer hit testing must update both the displayed title and the load request's selected track. These checks do not establish real Apple sign-in or the final GPU/audio output.

## Queue, media controls and lifecycle

The daemon owns MPRIS. `tests/transport.rs` covers transport and queue-context decoding with private-bus synthetic inputs. A context must preserve the original track order, selected row, initial position and play/pause intent in one operation.

Desktop code forwards activation and application links to the existing UI, distinguishing search navigation from track/context playback. Ordinary close/Quit should end playback unless explicit tray/background mode is selected; a view change to the mini player should not quit. A standalone daemon remains a separate user choice.

These are implementation contracts. The installed desktop's window-close, reopening, transport and context cases require runtime acceptance; consult the [desktop lifecycle plan](../specs/desktop-lifecycle-update/plan.md) and [Roadmap](roadmap.md) for verified and outstanding cases. Do not claim an installed result merely from synthetic tests.

## UI localization

The UI follows the system locale by default, with a persistent per-user setting under **Settings → Language**. Gettext catalogs are stored in `crates/melba/assets/i18n/`:

`de-DE`, `es`, `fr`, `it`, `ja`, `ko`, `nl`, `pl`, `pt-BR`, `pt-PT`, `ru`, `sv`, `tr`, `uk`, `zh-Hans`, `zh-Hant`

Missing keys fall back to English. The template is generated from the listed
sources and the catalogs are merged against it:

```sh
cd crates/melba
xgettext --language=C --from-code=UTF-8 -kgettext:2 -kngettext:2,3 -kpgettext:2c,3 \
  --no-wrap -o assets/i18n/melba.pot -f assets/i18n/POTFILES
for f in assets/i18n/*.po; do msgmerge --no-wrap --no-fuzzy-matching --update "$f" assets/i18n/melba.pot; done
cargo test --test localization
```

Keep every msgid and msgstr on one line: the PO library used by that test reads
a wrapped entry (`msgid ""` plus continuation lines) as an empty msgid, which
made a correct catalog look like it had a missing message. `POTFILES` must list
only files that exist; a removed source file made the recipe fail outright.

`catalogs_cover_the_template_and_preserve_named_placeholders` checks every
catalog against the template and names any difference. `COMPLETE` lists the
catalogs that must translate every message with no English fallback; it is
`ja` today, and a catalog must not be added there until its gaps are filled.
The other catalogs carry English fallbacks by design, so listing them there
would be a claim the files do not support.

## Quality, licensing and release gates

- `tests/source_format.rs` guards source encoding and ALAC codec-specific rate/depth/channel extraction.
- `evidence/slice01-format.json` records a path-specific negotiated-output fixture, not every device or track.
- Installers and binary packages must carry the generated, verified dependency notices as described in [NOTICE.md](../NOTICE.md).
- For any live sign-in or service change, keep account tokens out of logs/tests and separate fake-SDK coverage from credentialed acceptance.
- Before a public release, review [Publication scope](../PUBLICATION.md), signing provenance, hardware-specific gates and the remaining items in [Roadmap](roadmap.md).

Small, focused, tested patches are preferred. The desktop layout was inherited from Spotifast, and a native redesign suitable for Apple Music remains a welcome contribution. See `crates/melba/CONTRIBUTING.md` if present in the current checkout and [Credits](../README.md#credits-and-license).
