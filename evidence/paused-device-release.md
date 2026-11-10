# A paused player releases the output device (2026-11-11)

`melbad serve --paused` used to open the exclusive PCM and pause it, so the card
stayed reserved for as long as the desktop was open. The player now holds the
*selection* and opens the device only when a command plays something.

## What was measured

Host: the same machine, one USB DAC (`hw:0,0`) that no other process was using at
the time (`fuser /dev/snd/pcmC0D0p` empty before each run). Each case started a
fresh daemon with an isolated `XDG_*` root under `dbus-run-session`, loaded one
fixture through the bus with `MelbaLoadQueue(uris, 0, 0, false)` — a paused load —
and then asked it to play.

| build | after paused load | after `Play` |
| --- | --- | --- |
| v0.3.27 (before) | `fuser` → `2784884`, `state: PREPARED` | `fuser` → none, `closed` |
| this build (after) | `fuser` → none, `closed` | `fuser` → `2786190`, `state: RUNNING` |

The `status` file does not exist at all in the "after, paused" case: no substream
was ever allocated, which is stronger than "opened and idle".

## What this does not establish

* Only this host's USB DAC path was measured. The HDMI card (`hw:1,7`) was busy
  with another process during the run, so reading its status would have described
  somebody else's stream — it was not used.
* The queue-load path is what was measured. `ctl load` (the CLI, which plays)
  still opens the device, correctly.
* The in-repo gate for this is `tests/transport.rs::paused_startup_takes_no_output_device`,
  which reads `melba:streamOpens` from the bus (0 while paused, >0 after Play) and
  fails when the deferral is disabled.

## Reading this alongside the earlier attempt

An earlier attempt (2026-11-09, reverted) also released the device while paused,
but broke three transport tests. The cause was not what it first appeared to be:
`Prev` onto an unopenable source walked forwards again, `mpris:trackid` only moved
when something played, and re-loading an identical queue wiped the play history.
All three are fixed, which is what made this measurement possible.
