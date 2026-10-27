# Integrations

[README](../README.md) · [Audio and playback](audio.md)

## Sidra hand-off (experimental)

`tools/sidra-bridge.py` can hand playback from [Sidra](https://github.com/wimpysworld/sidra), an Electron wrapper of `music.apple.com`, to Melba's `melbad` engine while leaving Sidra's interface visible.

Sidra's MPRIS `mpris:trackid` includes catalog IDs of the form `/org/sidra/track/<id>`. The bridge translates them to `stream:` (subscription tracks) or `preview:` (30-second previews), mutes Sidra through its writable Volume property, mirrors Play/Pause/Stop, forwards `Seeked` and resynchronizes position drift greater than 1.5 seconds when media lengths agree.

```sh
melbad serve &
python3 tools/sidra-bridge.py --scheme stream   # use preview for unauthenticated previews
```

A live Sidra `OpenUri` probe reported `Playing` with a valid catalog track ID; the bridge offloaded playback and `melbad` returned a `BITPERFECT` verdict on direct `hw:` while Sidra was muted. That receipt belongs to the tested route, not every possible hand-off.

Full `stream:`/ALAC hand-off requires usable sessions on both ends. Sidra's Electron profile can also hold a `music-user-token`, but token handling inherits the local browser-session limitations described in [Authentication](authentication.md). Never copy or publish session databases or account tokens.

This integration is optional, not part of the normal Melba installation or a replacement for first-run authentication.
