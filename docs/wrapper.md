# Wrapper (ALAC decrypt service) control

Melba does **not** download, install, bundle or redistribute a wrapper. This
document covers only an already-installed, user-supplied service. The payload
and its terms are outside this repository (see [NOTICE.md](../NOTICE.md) and
[PUBLICATION.md](../PUBLICATION.md)).

## Modes

| Config | Meaning |
|---|---|
| `"wrapper_mode": "managed"` | Melba starts and stops an **already installed** managed service on demand. It does not install or fetch it. |
| `"wrapper_url": "tcp://…"` | Melba connects to a user-run service; no process control. |
| neither | An installed managed service is detected and started automatically; otherwise ALAC falls back to AAC. |

`managed` means process ownership, not installation. A staged managed
service is also detected when the key is absent, so the bootstrap can be
installed first and used without editing config.

## Supported service

The managed path uses the upstream **WorldObservationLog/wrapper** bundle that
Melba stages in its managed directory. It is the only service exercised with
the GUI native authentication bridge. Other services are not drop-in
equivalents:

| Service | Melba compatibility |
|---|---|
| WorldObservationLog/wrapper (managed) | supported managed path; GUI native login ABI |
| WorldObservationLog/wrapper-lite (HTTP key) | playback via `wrapper_url`; GUI native login not established |
| glomatico/wrapper-v2 (TCP decrypt) | playback via `wrapper_url`; GUI native login not established |
| aria / downloader suites | download pipelines, not a live decrypt endpoint for Melba |
| self-made protocol shell | playback possible; native GUI login ABI is missing |

Acquiring or installing the service is explicit and never automatic. The
source-only bootstrap is at
<https://github.com/nekomario28/melba-wrapper-bootstrap>; it fetches the
upstream bundle locally and does not redistribute it. This table is about
compatibility, not permission. That project has its own README, license and
security notes.

## Runtime flow

1. Opening an ALAC stream checks the service's key port.
2. If managed and the port is down, Melba starts the installed service and
   waits for the port. If it is missing or exits early, playback falls back to
   AAC and records the reason.
3. Sign-in stays in the GUI. With a compatible managed service, the native
   prompt is relayed through the private GUI auth path; otherwise the browser
   flow signs in for AAC. There is no separate CLI login step.
4. `--require-bitperfect` and the output picker are independent of the wrapper;
   they only decide whether the ALSA path is exact and shared or exclusive.

## Commands

```sh
melbad wrapper status   # installed directory, binary, key-port reachability
melbad wrapper start    # start the installed service (debug/foreground parent)
melbad wrapper stop     # stop the managed processes and verify the key port
melbad auth status      # local token presence/expiry; never prints the token
melbad setup            # audio output only; does not touch the wrapper
```

Use `melbad ctl ...` or MPRIS for transport; do not edit live account or
wrapper databases.

## Troubleshooting

- `running: no` with a reachable key port means another service owns it;
  Melba will not kill a service it does not manage.
- `complete GUI sign-in before using the managed wrapper` means the cached
  binding is a tombstone; finish the GUI sign-in or run `melbad auth clear`.
- `wrapper exited early` is recorded in the managed `wrapper.log`; inspect
  lifecycle lines, never paste account identifiers, tokens or signed URLs.
- If ALAC fails, playback reports the fallback and continues with AAC.

## Boundaries

- Bind the service to loopback; its ports are typically unauthenticated.
- Account and wrapper databases are local, owner-only files; do not attach
  them to diagnostics or logs.
- The service's license, Apple's terms, and Melba's licenses are separate.
- Melba contains the client/protocol and ALAC decode path, not the FairPlay key
  derivation; do not describe an installation as legal clearance.
