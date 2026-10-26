# Agent-native loop — verified 2026-10-16, hw:0.0

## pipeline
aml mk search "toto africa" --types songs --limit 3 --collection
  -> sone.collection/v1, 3 tracks, playUri=preview:185717604 ...
aml serve /tmp/toto-coll.json   (also: cat file | aml serve /dev/stdin — both verified)
  -> playback thread resolves preview:<id> via musickit::download_preview
  -> ALSA hw:0,0, verdict BITPERFECT, durationMs=30023
aml now
  {"status":"Playing","positionMs":1656,"durationMs":30023,"title":"Toto — Africa",
   "queue":["preview:185717604",...],"queueIndex":0,"shuffle":false,
   "loopStatus":"None","sink":"hw:0,0","verdict":"BITPERFECT"}
aml ctl pause -> Paused posMs 3059 ; ctl play -> Playing posMs 4600
aml mk collection albums 1440903617 -> "13 Ways to Bleed Onstage", 13 tracks,
  props: apple.audioVariants="lossless,lossy-stereo", apple.artwork, apple.catalogId

## notes
- title map: collection track `artist — title` seeds MPRIS xesam:title (was raw catalog id before).
- pausing immediately after ctl is a race; state settles within ~1s (verified).
- `aml about` prints the capability manifest for agent self-discovery.
