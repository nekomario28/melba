# Browser-storage fixture

`chromium-localstorage/` is a minimal database captured from a fresh Vivaldi
8.2.4133.89 Chromium-mode headless profile. A loopback page wrote a deliberately
unsigned fake `media-user-token`; no Apple page, real account, or existing user
profile was used. The origin is `http://127.0.0.1:32821`, not Apple.

The parser test first requires that the real browser record can be recovered
and decoded. Only then does it check the origin boundary: an unreadable or
empty database must not masquerade as successful rejection. Keep CURRENT,
MANIFEST, and log together. Other tests generate coherent LevelDB databases to
exercise Apple-origin acceptance, overwrites, deletion, compaction, and encoding.
