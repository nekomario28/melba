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
  explicitly (so the new guard cannot apply), the full `melbad --lib` suite
  left the live file byte-identical.
- The **desktop crate's test run is not**: with the same variable set, running
  `cargo test --manifest-path crates/melba/Cargo.toml` changed the live file to
  the `["stream:b","stream:a"]` shape.
- Removing the offending test in October did not remove the writer. The queue
  was destroyed again, so the class needs a structural guard rather than
  another deletion.

## Guard added

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

## Still open

The desktop crate reaches a **live** daemon in two ways that this guard does
not cover:

- `daemon_command` falls back to a `melbad` found on `PATH` when no sibling
  binary sits next to the executable. On this machine that resolves to
  `~/.local/bin/melbad`, the user's installed daemon.
- Integration tests isolate some XDG variables and not others
  (`tests/branding.rs` sets `XDG_CONFIG_HOME` alone at one call site), so the
  daemon they start inherits the real state directory.

The fix is for the app to pass its own resolved directories to the daemon it
starts, so the pair always agrees on where state lives, and for every test that
starts a binary to isolate all four XDG variables. Not applied yet: it changes
the daemon-start contract and needs its own verification.

## Recovery

The original queue is **not recoverable**. `session.json`'s `last_queue_rows`
is empty (the app rewrote it while restoring the synthetic queue), so the
30-row partial recovery used in October does not exist this time.
`history.json` still holds 105 real plays, which can rebuild a queue by
replaying them; that is a new queue, not the old one.
