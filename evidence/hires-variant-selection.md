# Which ALAC rendition a master actually offers (2026-11-11)

Measured read-only against the catalog with the account's own stored token, using
`cargo run --manifest-path crates/melba-cdm/Cargo.toml --bin diag 268443097`
("So What", Kind of Blue — an album whose catalog entry lists
`hi-res-lossless`).

## The master, as served

`enhancedHls` → `https://aod.itunes.apple.com/itunes-assets/HLSMusic221/…/P1305215351_default.m3u8`

```
variant codecs=mp4a.40.2 bandwidth=268877
variant codecs=mp4a.40.2 bandwidth=268869
variant codecs=mp4a.40.2 bandwidth=267935
variant codecs=alac      bandwidth=4995600   ← gr9216_alac.m3u8
variant codecs=ec-3      bandwidth=772666
variant codecs=ec-3      bandwidth=452062
variant codecs=alac      bandwidth=1761161   ← gr2304_alac.m3u8
variant codecs=mp4a.40.2 … (five more, all under 300 kbit/s)
```

Two things follow, and the second is a defect:

1. A master does state the number that separates the renditions — `BANDWIDTH` per
   variant — so `hi-res` can be a real choice rather than a wish.
2. **Apple does not list the lossless rendition first.** The first ALAC variant in
   this master is the 4,995,600 bit/s one. Choosing "the first ALAC" — what the
   player did before this change, and what `pick_alac_variant(.., false)` did
   while it was written that way — therefore asked for the **hi-res** rendition
   under the `lossless` setting.

## Other masters, to see what varies

The same read on two more albums, one advertising `lossless` and one
`hi-res-lossless`:

| album | catalog entry says | ALAC variants in the master | both settings ask for |
| --- | --- | --- | --- |
| Random Access Memories (`617154241`) | `lossless, lossy-stereo` | one, 1,717,091 (`gr2116`) | that one |
| Blue Train (`1468202477`) | `hi-res-lossless, lossless` | one, 1,969,779 (`gr2304`) | that one |
| Kind of Blue (`268443092`) | `hi-res-lossless, lossless` | two, 4,995,600 (`gr9216`) and 1,761,161 (`gr2304`) | different ones |

So the number of ALAC renditions is not fixed: with one rendition the setting
changes nothing (as it should — there is nothing higher to ask for), and the album
that advertises hi-res does not always serve two renditions for a given song.
This is why the choice reads `BANDWIDTH` per master instead of assuming a shape.

## What each setting asks for now

| setting | variant | bitrate |
| --- | --- | --- |
| `lossless` | `gr2304_alac.m3u8` | 1,761,161 |
| `hi-res` | `gr9216_alac.m3u8` | 4,995,600 |

Same two values the master states, now chosen by the numbers instead of by
position. `crates/melba-cdm/src/stream.rs::a_real_master_is_ordered_hi_res_first`
pins exactly this variant list, so the ordering cannot quietly become an
assumption again.

## What the two renditions actually are

The init segment of each rendition is readable before decryption, so `ffprobe` on
the cleaned init says what the numbers mean:

| rendition | bandwidth | codec | rate | depth | channels |
| --- | --- | --- | --- | --- | --- |
| `gr2304_alac.m3u8` | 1,761,161 | alac | 48,000 | 24 | 2 |
| `gr9216_alac.m3u8` | 4,995,600 | alac | 192,000 | 24 | 2 |

Both are 24-bit; the difference is the sample rate. That matches Apple's own two
tiers — *Lossless* up to 24-bit/48 kHz, *Hi-Res Lossless* above — so the mapping
`lossless` → 24/48 and `hi-res` → 24/192 is the service's distinction, not an
invented one.

For reference, an album with a single ALAC rendition (Random Access Memories,
`617154353`) serves one 24-bit/44.1 kHz variant, where the setting changes nothing.

## Why `auto` still asks for the hi-res rendition

Until v0.3.28 the player took the **first** ALAC variant, which is the hi-res one
here. Choosing "the lowest bitrate" for every non-hi-res setting would therefore
have quietly downgraded 24/192 to 24/48 for anyone who never opened Settings —
a change nobody asked for, in a player whose claim is exactness. So:

| setting | asks for | why |
| --- | --- | --- |
| `auto` | the best the master offers (here 24/192) | keeps the behaviour v0.3.28 had |
| `lossless` | the base tier (here 24/48) | an explicit request, e.g. for less bandwidth |
| `hi-res` | the best (here 24/192) | what the name says |

`the_automatic_choice_keeps_asking_for_the_best_rendition` pins that mapping.

## What this does not establish

* **No audio was decoded.** Both renditions need the user's decrypt service, which
  was not started for this measurement; what is verified is which playlist the
  player requests, not what the samples contain.
* The parsed init header of the hi-res rendition reads `rate=1`, `bits=16`; that
  is the QuickTime sample-entry rate field (1.0 as a 16.16 fixed-point number),
  not the real rate. The authoritative rate lives in the ALAC magic cookie, which
  the decoder reads. Recorded here so the number in a future diagnostic is not
  mistaken for a regression.
* One song from one album. The ordering seen here is not proven to be a rule
  Apple follows everywhere, which is exactly why the choice reads `BANDWIDTH`.
