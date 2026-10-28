# User state incident: the live queue was overwritten by tests, again

2026-10-28. `~/.local/state/melba/queue.json` held two synthetic queues at
different times during test runs, not the user's queue:

- `{"idx":2,"queue":["stream:m0","stream:new"]}` — the `melba:track:m0` /
  `melba:track:new` fixtures used by the desktop crate's own tests.
- `{"idx":0,"queue":["stream:b","stream:a"]}` — the same `a`/`b` shape the
  2026-10-26 incident recorded
  (`~/.cache/melba-user-state-incident-20261026/queue.json.destroyed-by-test`).

The daemon stores queue entries as `stream:<id>`, so these are
`melba:track:m0` / `melba:track:a` test fixtures that reached a real daemon.

## What is established

- The **root crate's unit tests are clean**: with `XDG_STATE_HOME` set
  explicitly (so the guard cannot apply), the full `melbad --lib` suite left
  the live file byte-identical.
- **The desktop crate's tests are clean too.** This corrects an earlier entry
  in this file. The file changed while a desktop test run was in progress, and
  that was read as the tests writing it — but the user's app and daemon were
  running throughout, and they persist the queue themselves. Re-measured with
  the live app stopped: the full desktop suite (`--all-features`, no XDG
  isolation at all) leaves the live file byte-identical, and it does so **even
  with the daemon-directory hand-off below disabled**. The earlier observation
  was confounded by a concurrent writer; it is not evidence about the tests.
- **The writer is therefore not established.** The two synthetic queues
  (`stream:m0`/`stream:new`, `stream:b`/`stream:a`) are fixture-shaped, so
  something ran the app or daemon against the live state directory. The
  candidates are a manual `--demo` run, a screenshot/headless run, or the
  daemon restoring a queue another process had already replaced. None is
  proven, and this file must not name one until it is.
- Removing the offending test in October did not remove the writer, and the
  queue was destroyed again. Whatever the writer is, it is still at large.

## Guard added (root crate)

`melbad`'s `config::xdg_dir` — the one leaf every module resolves user
directories through — now resolves an **unset** variable inside a per-process
temporary root under `cfg(test)`. A test can no longer write the user's
configuration, state or cache by forgetting to isolate the environment; a
harness that wants a real directory still sets it explicitly, which the guard
honours.

`config::tests::tests_never_resolve_a_user_directory_by_default` pins the
shape, and it is proven able to fail: with no guard the resolution returned
`$HOME/.local/state/melba/queue.json`.

The root is keyed by thread, not by process: the harness gives each test its
own thread, so two tests that persist state never share a file while one test
sees a stable path across every call. A process-wide root made
`wrapper::process::tests::isolated_owner_death_reaps_workers_and_releases_the_session_lock`
fail on the shared session lock, which is how the choice was checked rather
than assumed.

## What was ruled out by experiment

Each candidate was run with the live daemon **running**, because the queue can
only be written by a daemon and only a run that reaches one can replace it.

| Candidate | Live queue |
|---|---|
| Full desktop suite, no isolation at all | unchanged |
| The same suite with the directory hand-off disabled | unchanged |
| `cargo test --test startup_diagnostics` (runs `--demo --demo-shot`) | unchanged — the test does not execute (0 tests run) |

The first two were also run with the live app **stopped** and were unchanged
there too, which is how the earlier "the tests did it" reading was falsified:
stopping the app removes the daemon, and with it the only writer. The installed
release build has no `--demo` flag at all (it is behind the `demo` feature), so
a demo run against the live profile needs a development build.

None of these names the writer, and thinking it had been named is what cost
this investigation its time.

## Recording the next occurrence

`persist_queue` now appends one line to `queue-writes.jsonl` beside the queue:
pid, ppid, executable, argv, `XDG_STATE_HOME`, `HOME`, and the queue's length,
index and first entry. Each earlier occurrence had to be guessed from a file
timestamp; this makes the next one attributable. It is best effort — a failure
to record never affects playback — and one line per queue mutation, not per
packet. `player::tests::a_queue_write_records_who_wrote_it` pins the fields,
including that a second write appends rather than replaces.

## Contract change (desktop crate)

The app now passes its own resolved directories to the daemon it starts
(`DaemonDirs`), so the pair always agrees on where state lives instead of
inheriting the environment, and every test that starts a binary isolates all
five variables (HOME plus the four XDG bases). A unit test pins the hand-off,
including that an unset directory is not passed as an empty value.

This removes a real way for a test or a stray run to point a real daemon at the
user's files. It is **not** claimed as the fix for the destruction above: the
falsification run shows the tests were not writing the file in the first place.

## Recovery

The original queue is **not recoverable**. `session.json`'s `last_queue_rows`
is empty (the app rewrote it while restoring the synthetic queue), so the
30-row partial recovery used in October does not exist this time.
`history.json` still holds 105 real plays, which can rebuild a queue by
replaying them; that is a new queue, not the old one.
