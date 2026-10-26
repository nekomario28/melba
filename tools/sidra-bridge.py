#!/usr/bin/env python3
"""Sidra -> melbad playback hand-off bridge.

Sidra (Electron wrapper of music.apple.com) exposes a bidirectional MPRIS
service whose `mpris:trackid` embeds the Apple Music catalog song id
(`/org/sidra/track/<catalogId>`). This daemon watches that surface and hands
the real audio output to `melbad serve` — which can play the same track through
the native bit-perfect / ALAC path — while Sidra keeps running muted so its
UI, progress bar and transport controls stay alive.

Flow:
  new Sidra trackid  -> `melbad ctl open <scheme>:<catalogId>` + Sidra Volume=0
  Sidra Seeked       -> `melbad ctl seek <us>`
  Sidra status flip  -> Play/Pause/Stop mirrored to melbad
  periodic           -> if positions drift > DRIFT_S, re-seek melbad

Usage: sidra-bridge.py [--scheme stream|preview] [--dry-run] [-v]
  --scheme preview plays 30s non-DRM previews (no subscription token needed)
  so the whole mechanism can be exercised before sign-in exists on both ends.
"""

import argparse
import re
import subprocess
import sys
import time

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

SIDRA = "org.mpris.MediaPlayer2.sidra"
PATH = "/org/mpris/MediaPlayer2"
PLAYER = "org.mpris.MediaPlayer2.Player"
PROPS = "org.freedesktop.DBus.Properties"
NO_TRACK = "NoTrack"
DRIFT_S = 1.5
POLL_S = 3.0

bus = None  # created in main() after the GLib mainloop is installed
args = None
last_catalog = None
last_status = None
offloaded = False  # whether melbad currently owns the audible stream


def log(*a):
    if args.verbose:
        print("[bridge]", *a, flush=True)


def sidra_prop(name):
    p = dbus.Interface(bus.get_object(SIDRA, PATH), PROPS)
    return p.Get(PLAYER, name)


def sidra_set(name, value):
    p = dbus.Interface(bus.get_object(SIDRA, PATH), PROPS)
    p.Set(PLAYER, name, value)


def sidra_call(method, *a):
    p = dbus.Interface(bus.get_object(SIDRA, PATH), PLAYER)
    getattr(p, method)(*a)


def melbad(*argv):
    cmd = ["melbad", "ctl", *argv]
    log("$", " ".join(cmd))
    if not args.dry_run:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            log("melbad err:", r.stderr.strip() or r.stdout.strip())
        return r.returncode == 0
    return True


def catalog_of(meta):
    """MPRIS Metadata -> song catalog id; None when no track / unresolvable.

    mpris:trackid is Sidra's internal object id — songs usually appear as
    /org/sidra/track/<digits> but also surface as i_… ids (Sidra's player
    item id, seen on real streams). xesam:url always carries the real
    catalog URL (…/song/<digits>), so prefer that when the trackid tail
    is not a plain song id."""
    if not meta:
        return None
    tid = str(meta.get("mpris:trackid") or "")
    if not tid or NO_TRACK in tid:
        return None
    tail = tid.rsplit("/track/", 1)[-1]
    if tail.isdigit():
        return tail
    url = str(meta.get("xesam:url") or "")
    m = re.search(r"/song/(\d+)", url)
    if m:
        log(f"trackid {tail} is not a song id — using {m.group(1)} from xesam:url")
        return m.group(1)
    return tail if tail and tail != "unknown" else None


def silence_sidra():
    """Keep Sidra 'playing' for UI/progress but silent. Volume is writable."""
    try:
        sidra_set("Volume", dbus.Double(0.0))
        log("sidra volume -> 0 (muted, UI stays live)")
    except dbus.DBusException as e:
        log("volume set failed:", e)


def on_props_changed(iface, changed, invalidated):
    global last_catalog, last_status
    if iface != PLAYER:
        return
    if "Metadata" in changed:
        cat = catalog_of(changed["Metadata"])
        if cat and cat != last_catalog:
            last_catalog = cat
            offload(cat)
    if "PlaybackStatus" in changed:
        st = str(changed["PlaybackStatus"])
        if st != last_status:
            last_status = st
            mirror_status(st)


def offload(catalog):
    global offloaded
    # `i_…` ids are music videos — melbad only resolves numeric song catalog
    # ids, and a video can't be ALAC anyway. Leave Sidra playing it.
    if catalog.startswith("i_"):
        log(f"{catalog} is a music video — staying on Sidra")
        return
    print(f"[bridge] new track {catalog} -> melbad ({args.scheme}:)", flush=True)
    if melbad("open", f"{args.scheme}:{catalog}"):
        offloaded = True
        # mute Sidra only once melbad actually reports Playing — otherwise a
        # failed/cold open would leave BOTH sides silent
        GLib.timeout_add(2000, mute_when_playing, [15])
    else:
        log("melbad open failed — leaving Sidra audible")


def mute_when_playing(retries):
    st = melba_prop("PlaybackStatus")
    if st is not None and str(st) == "Playing":
        silence_sidra()
        return False
    retries[0] -= 1
    if retries[0] <= 0:
        # open() succeeded but Playing is still not here — keep offloaded;
        # poll() will mute Sidra the moment melbad reaches Playing, so a slow
        # DRM/wrapper cold start can't leave both sides audible forever
        log("melbad still not Playing — poll() will mute on late start")
        return False
    return True  # re-arm


def mirror_status(st):
    if not offloaded:
        return
    print(f"[bridge] sidra {st} -> melbad", flush=True)
    melbad({"Playing": "play", "Paused": "pause", "Stopped": "stop"}.get(st, "status"))


def on_seeked(pos_us):
    if offloaded:
        log("seek", pos_us)
        melbad("seek", str(int(pos_us)))


def melba_prop(name):
    try:
        p = dbus.Interface(
            bus.get_object("org.mpris.MediaPlayer2.melba", PATH), PROPS
        )
        return p.Get(PLAYER, name)
    except dbus.DBusException:
        return None


def poll():
    """Re-sync melbad to Sidra's position when they drift apart — but only
    when both sides play the same-length media (a 30s preview on one side
    and a 90s/full track on the other can never be aligned)."""
    if offloaded and last_status == "Playing":
        try:
            sp = int(sidra_prop("Position"))
            sl = int(sidra_prop("Metadata").get("mpris:length", 0))
            sv = float(sidra_prop("Volume"))
        except dbus.DBusException:
            return True
        # late-arriving Playing: mute_when_playing's window elapsed but melbad
        # got there eventually — Sidra must not stay audible alongside melbad
        if sv > 0 and str(melba_prop("PlaybackStatus")) == "Playing":
            silence_sidra()
        ap = melba_prop("Position")
        al = 0
        m = melba_prop("Metadata")
        if m:
            al = int(m.get("mpris:length", 0))
        if ap is None or (sl and al and abs(sl - al) > 2_000_000):
            return True  # different media lengths -> positions incomparable
        drift = (sp - int(ap)) / 1e6
        if abs(drift) > DRIFT_S:
            log(f"drift {drift:+.2f}s -> re-seek melbad")
            melbad("seek", str(sp))
    return True


def main():
    global args, bus
    ap = argparse.ArgumentParser()
    ap.add_argument("--scheme", default="stream", choices=["stream", "preview"])
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    bus.add_signal_receiver(
        on_props_changed,
        signal_name="PropertiesChanged",
        dbus_interface=PROPS,
        bus_name=SIDRA,
        path=PATH,
    )
    bus.add_signal_receiver(
        on_seeked,
        signal_name="Seeked",
        dbus_interface=PLAYER,
        bus_name=SIDRA,
        path=PATH,
    )
    GLib.timeout_add_seconds(POLL_S, poll)
    print(f"[bridge] watching {SIDRA} ({args.scheme}: offload)", flush=True)
    # initial state: a track may already be playing when the bridge starts
    try:
        meta = sidra_prop("Metadata")
        cat = catalog_of(meta)
        st = str(sidra_prop("PlaybackStatus"))
        if cat:
            last_catalog = cat
            last_status = st
            offload(cat)
    except dbus.DBusException as e:
        log("initial read failed:", e)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
