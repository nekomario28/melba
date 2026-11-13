# Responsive selection

Two independent user contracts (2026-11-13):

1. `MelbaPlayQueued(row, uri)` selects an occurrence directly with one GUI / worker / D-Bus command, preserving context, shuffle tail and duplicate occurrences. Never implement it as repeated Next.
2. Manual Next / Previous share directional navigation and supersede older source work, including mixed-direction bursts. Intermediate preparations may be discarded. Do not wait for an old response or add a debounce delay before accepting the newer selection.

## Next Agent Prompt

Both contracts are implemented and locally verified. Source navigation uses one directional-skip path during preparation, cold pause and packet playback. Next / Previous also invalidate older GUI context-resolution results. Preserve both contracts; do not reintroduce repeated-Next queue selection. After explicit user authorization, both binaries were installed locally and the installed paths passed scoped silent runtime tests. The test namespace hides sound/GPU devices and the normal session bus, blocks external networking, uses ALSA null and private XDG state. It checks both 32-press directions, mixed directions, direct selection and pause/resume, Stop/Quit, plus software-rendered GUI startup/IPC/exit. Signed-out GUI transport is intentionally gated and was NOT verified; no real account or physical output was used. The pre-existing normal processes were left running on their old executable inodes until the user restarts. The user subsequently reported no issues with the installed build and authorized a branch push. No specific interactive or listening sequence was recorded with that acceptance. Remote CI and independent review remain outstanding; release publication has not been requested.

- [x] Direct queue selection and regression against repeated Next.
- [x] Cancellation primitive: isolated per-operation pooled HTTP agents, immediate shutdown, scoped propagation and resource deregistration.
- [x] Integrate independent command ingress, cancellable file reads / wrapper TCP and source checkpoints; reject stale completions without fallback or output.
- [x] Prove latest selection, Stop and Quit before releasing an old stalled source; preserve pause, shuffle, duplicate history and command ordering; run existing suites.

## Local verification

The matching-source run passed 193 workspace tests plus the thread-bound scope compile-fail doctest; GUI default 800 and all-features 832 tests; root and GUI Clippy with warnings denied, formatting and diff checks. Transport fixtures require the final source to be ready before releasing the obsolete FIFO, including 32 backward presses and alternating directions. Unit tests pin the three-second restart rule, mixed-direction history and command barriers. The real HTTP body test consumes the response prefix before cancellation, distinguishing body cancellation from header cancellation.

Removing ingress cancellation, excluding Previous from cancellation, and removing HTTP shutdown each produced the intended assertion failure. A stale GUI context result after Next also failed before the shared epoch fence was applied. The latest HTTP mutation was restored and followed by all 193 workspace tests and Clippy. Evidence and source/artifact hashes are recorded locally under `~/.cache/melba-queue-jump/responsive-verification.json`; no CI or hardware claim follows from this receipt.

## Architecture and reuse

The daemon remains the sole owner of queue order, decoder and ALSA. A command ingress thread may interrupt source I/O, but must not mutate the playback cursor or call ALSA. Keep worker count bounded; no thread or task per click. One operation owns its cancellation flag, HTTP pools and registered interrupt handles. Source boundaries consume queued operations before committing metadata / fallback / output. A new operation must not miss a selection queued concurrently with its creation.

Use the **already locked ureq 3.5.1** API `Agent::shutdown`, not a new async runtime or unsafe thread termination. Inspected donors: ureq `src/agent.rs` (`shutdown`, shared pool semantics), `src/shutdown_tests.rs` (real stalled header/body sockets), `src/unversioned/transport/tcp.rs` (registered socket shutdown). Normal unscoped calls retain the process-wide pool. Scoped calls reuse a pool within the operation, but cancellation cannot shut down unrelated catalog/auth work. Keep proxy, timeout, TLS and credential redirect policies unchanged.

Local readers use nonblocking FIFO open plus readiness/cancellation notification; the same reader must serve tag probing and source probing. Do not leave a blocking tag-reader bypass. Segment prefetch must explicitly inherit the operation. Preview downloads must not commit partial cache files on cancellation.

There is no UI layout change and no new compatibility layer / repeated-Next fallback. Keep the existing direct-selection protocol and its old-daemon error. Split the feature by API ownership, not parallel queue implementations. Keep verification bounded and reproducible rather than starting speculative workers.

## Gates and limits

- A blocked real HTTP header/body read must exit on cancellation without the test server releasing it; pooling and unscoped requests must survive unrelated cancellation.
- In the real transport fixture, hold the old source unreadable. After Skip / direct selection, require the final track to become ready **before** releasing the old source. Repeated obsolete requests must not create an increasing worker backlog.
- Stop / Quit must not wait for that old source. Cancellation is not a load error: do not advance unexpectedly, try a preview, overwrite newer metadata, or start output for the old source.
- Keep actual queue occurrence history, paused identity, frame accounting and all existing transport tests. Prove tests fail on old / mutated paths.
- OS-level DNS, connect establishment, ordinary filesystem kernel stalls, and native CDM calls are not magically interruptible by shutting down an established HTTP connection. Record unproven boundaries explicitly; do not claim an unconditional wall-clock guarantee for these native operations.
- Use isolated HOME/XDG / D-Bus / ALSA null and light-gated single-job builds with MemoryMax. No audible or hardware acceptance is implied.
