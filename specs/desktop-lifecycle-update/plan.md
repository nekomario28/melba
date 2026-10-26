# Desktop lifecycle and update UX

## User contract

- Install and activate the current local fixes on this PC through CLI/IPC, without desktop input injection or authentication-file edits.
- Ordinary window close and explicit Quit end the app and its playback daemon. An explicitly enabled background/tray preference remains intentional hiding, not quitting. Replacing the main window with the mini player must not stop playback.
- Starting the real app starts its local daemon when needed. Demo/headless clients never start or stop the user's service.
- Update controls must produce real check/support/download/install results, not fabricated “no update” replies or permanent spinners. Keep publisher signatures mandatory. Never silently update from another product's repository.

## Slices

1. **Lifecycle:** one worker-owned start/stop path, default close exits, existing explicit background mode respected. Add CLI Quit for the same app action. Test startup/shutdown with private buses and process fixtures, not the user's service.
2. **Desktop handoff:** the daemon is Linux's sole MPRIS owner. Forward Raise and application/context links to the existing frontend's CLI/IPC, so removing duplicate frontend ownership does not lose activation/link handling. Do not launch a window from inside the daemon's systemd cgroup.
3. **Update adapter:** connect the existing signed updater to the worker's command/events on background tasks. Test an actual loopback feed both directly and through the worker; prove the old stub fails. Failed checks/unsupported installations must be visible and terminate their pending state. Offline demo mode may use an explicit local feed, never GitHub.
4. **Delivery:** affected/full private suites, independent scoped review, release build under the existing resource gate, install both binaries, then CLI/IPC identity and lifecycle verification. Preserve unrelated dirty changes. Do not claim live playback from widget tests or a successful method reply.

## Distribution boundary

The configured release channel is currently source-only. The upstream updater replaces one executable; it does not provide a paired melba/melbad transaction. A complete client self-update must not leave incompatible daemon and frontend binaries. Do not weaken signatures or invent a successful update to paper over missing distribution assets. Signed paired-package publication/rollback remains a separately gated release step unless a compatible existing installer can be reused. No release uploads or signing-key disclosure are authorized by this implementation plan.

## Non-goals

No authentication redesign, quality preferences, new UI styling, new audio sources, desktop automation, unrelated refactor, or forced 96 kHz output. Additional UX fixes must be directly exposed by these flows and get a regression guard.
