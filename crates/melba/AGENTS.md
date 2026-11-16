# Melba agent guide

Follow [CONTRIBUTING.md](CONTRIBUTING.md) for product and contribution policy.
These instructions add implementation constraints for coding agents. Paths in
this file are relative to this desktop crate unless marked repository-root.

## Product boundaries

- Keep Melba a small native Apple Music client on the `melbad` daemon. Do not
  replace the native interface with a web app, add telemetry, a hosted backend,
  or alternate sources for Apple Music audio. The daemon's existing sign-in-only
  webview is not a general browser or a playback frontend.
- Playback capabilities come from the `melbad` daemon. Do not advertise or
  implement a capability merely because its name appears in an API response
  or enum. In particular, do not pursue DRM circumvention.
- Do not broaden a task into adjacent features or a general refactor. Preserve
  existing user behaviour unless the task explicitly changes it.
- Do not add a setting without the maintainer's approval in the issue first;
  prefer a good default, a shortcut or a command (`CONTRIBUTING.md`, Settings).

## Architecture

- `src/ui/` draws views and emits `Action`s. Apply actions after drawing in
  `src/app.rs`; do not mutate application state from inside a borrowed view.
- Network and playback work belongs on the runtime in `src/backend.rs` and
  `src/daemon_worker.rs`, never as blocking work on the UI thread.
  `src/player.rs` holds the playback types the UI and the worker share.
- File writes that wait for the disk (anything through `fastframe_file`)
  never run on the UI thread: encode there and hand the bytes to the app's
  `fastframe_file::Writer`. fastframe_file's guard panics in debug builds.
- Keep platform integrations behind target-specific modules or `cfg` blocks.
  A fix for one platform must keep the other two targets compiling.
- Settings and state files must remain readable, backward compatible, and
  atomically written. Never log credentials or authorization responses.
- Prefer existing dependencies. Explain any new crate in `Cargo.toml` next to
  the dependency when the reason is not obvious.
- Follow the [dependency policy](CONTRIBUTING.md#dependencies). The existing
  `crmne` sources are pinned upstream dependencies, not repositories owned by
  this project's maintainer. Preserve the checked-in `vendor/fastframe-update`
  path dependency and its provenance; do not add forks or vendor more code
  without explicit approval.
- egui and winit come from forks shared with ZapFast, RekordFlash and TonePush
  (crmne/egui apps-0.36, crmne/winit apps-0.30); move all their crates to a
  new revision together. On Wayland, eframe from that fork paces frames by
  the compositor's frame callbacks instead of a vsync swap, so a hidden window
  cannot freeze the app (#266). Do not add a vsync decision of our own.
- The egui fork shapes right-to-left runs in their own direction but leaves
  them in logical order. Pass logical text to `crate::bidi`, which reorders
  the laid-out runs; never reorder the string before layout.

Read `src/daemon_worker.rs` and `src/mk.rs` before changing authentication,
MusicKit requests, credential storage, or network behaviour — the worker is
the whole contract between the UI and `melbad` (commands it answers, events
it emits, MPRIS surface it mirrors). Read the queue rules and the queue tests
in `src/app.rs` before touching the queue. Read the nearby module tests
before changing a state machine or API fallback.

Playback capability claims belong to the daemon, not the UI: what `melbad`
can play (local files, `preview:`, `stream:` AAC via `melba-cdm`, `stream:`
ALAC via a user-supplied wrapper) is described in the repository-root
[README](../../README.md) and [audio documentation](../../docs/audio.md).
Dated specs are not current capability acceptance. Do not advertise a capability
merely because an Apple Music API field names it.

The interface is optimistic, always. A control shows its result the
moment it is used: a double-clicked song is the playing song, Next pops
the queue's head, an added song has its row. The backend then makes it
true and the daemon's state catches up behind; an answer that still tells
the story from before the user's action is stale, so hold the shown
state and ask again rather than let the lagging answer undo what the
user just did. Nothing the user did may ever flicker away and come back.

## Runtime verification

- Use the shipped `melba` CLI and IPC for runtime actions, and `melbad now`
  for daemon readback. Do not inject desktop mouse or keyboard events unless
  the user explicitly requests that interaction. Exercise widget-only paths
  through the isolated headless tests instead.
- A successful command or changed title is not playback proof. Wait for the
  requested URI's opened-source format and advancing position. Distinguish
  per-stream output PCM from physically audible sound or bit-perfect output.

## Issue communication

- Read/check is not permission to post, edit, close or upload. External actions
  require explicit authorization for the current request. Do not send an
  unsolicited reply merely because a comment was inspected.
- Write public replies for the reporter, not as an engineering investigation
  log. Keep them short, direct, and in plain language.
- A reply should move the issue forward: make the maintainer's decision, say
  that a fix is planned or in progress, or ask for one specific thing needed
  next. Include technical detail only when the reporter needs it to act.
- When a valid issue has a clear, bounded fix that can be implemented now,
  implement it instead of posting the proposed design in the issue. Do not use
  public comments as notes to yourself or as a substitute for doing the work.
- When authorized to close a bug, wait until its fix is on `main` and the
  relevant checks pass. State which commit fixes it and whether it is released.
  Reporter confirmation is
  welcome, but is not a routine requirement for closure; reopen if the problem
  persists after updating. Keep an issue open when the fix is still uncertain
  or only part of the report has been addressed.
- Never post two maintainer comments in a row on the same issue or pull
  request. If nobody has replied since the last maintainer comment, edit that
  comment instead.
- Keep private investigation notes out of the public thread. Do not post a
  second comment merely to document more analysis.
- Never use em dashes. Use a full stop, comma, colon, or parentheses instead.

## Interface review

- Distinguish an internal UI refactor from an interface redesign. Moving
  navigation or controls, regrouping menus, changing the application shell,
  window chrome, panel ownership or sizing, responsive breakpoints, spacing,
  or visual hierarchy is a redesign even when behavior still works.
- Call out every user-visible interface change at the top of a pull request
  review. Correct code and green CI do not make a redesign merge-ready.
- Require explicit maintainer approval of the visual scope before merging an
  interface redesign. Conditional approval to assess code quality is not
  approval of changed appearance or interaction.
- Inspect before-and-after evidence at representative window sizes and in both
  light and dark themes. If that evidence is missing, request it.
- Keep visual evidence in the pull request, in the format under "Visual
  reviews" in `CONTRIBUTING.md`: matching captures attached to the description
  or a comment, Before and After side by side for both themes at narrow and
  normal sizes, and relevant interaction states. Attach an interactive
  comparison, or the index for a batch (PR numbers, a selector, Previous/Next
  controls and links to individual comparisons), as a self-contained HTML
  file, or publish it outside the repository, and link it. Never commit
  captures, comparison pages or recordings in this repository.
- Record approval and requested adjustments with the relevant PR or existing
  decision record. Do not assume an external triage ledger exists.
  Keep visual approval separate from outstanding implementation or test gates.
  Do not ask again for unchanged approved scope after a rebase. A concrete
  requested adjustment is authorization to make that adjustment and update its
  evidence; ask again only for scope beyond the approval or request.

## Melba-specific dependency policy

This repository belongs to `nekomario28`, not `crmne`. The
[local dependency policy](CONTRIBUTING.md#dependencies) takes precedence over
ownership assumptions in the inherited automation section below. Existing
upstream pins and reviewed vendored code are not permission to introduce a new
source or update a revision without approval.

<!-- github-automation: pull-requests -->
## Pull requests

This section is maintained account-wide by
[crmne/github-automation](https://github.com/crmne/github-automation) and is
replaced when that policy changes. Do not edit it here. If it does not fit this
repository, say so in a review or issue, and put repository-specific rules in a
separate section, which takes precedence.

Review evidence stays out of the repository. Before-and-after screenshots,
captures, comparison pages, recordings, and anything else made to show
reviewers a change go in the pull request description or its comments: attach
the files there, or link a page published elsewhere. Only media the project
itself uses, such as documentation figures, release-note images, and app
assets, belongs in the repository.

Dependencies come from their package registry, such as crates.io, RubyGems, or
npm, or from repositories the maintainer controls, under `github.com/crmne/`
or this repository's own organization. A git dependency, or a patch, override,
or replace entry, may point only at such a repository, and adding or changing
one needs the maintainer's explicit approval. Never point a dependency at a
contributor's fork or any other source the maintainer does not control. When a
dependency needs a fix, send it upstream and ask the maintainer to carry it in
a fork they control until a release includes it.

Reviewers treat either problem in a pull request as a blocker.
<!-- /github-automation: pull-requests -->

## Branches

The default branch is `main`; keep its history linear. Follow the
[contribution workflow](CONTRIBUTING.md#pull-requests): use a focused topic
branch and PR for authorized agent publication unless the maintainer explicitly
requests a different flow. A request to edit locally is not authorization to
push, create a PR, merge, tag or publish a release.

Keep commits focused and preserve contributor credit when squash-merging.
Do not add assistant-attribution trailers. Never create merge commits to bypass
the linear history. Use fast-forward-only pulls and rebase unpublished commits
if needed. Rewriting published history requires explicit approval and an exact
force-with-lease guard; keep a local recovery ref.

## Disk use

Builds go through [mbx](https://mr-boxington.jdx.dev), enabled for mise users
by `mise.toml` (run `mise trust` once in each new checkout or worktree, or
mise refuses to run `cargo` there). It keeps compiled work in one shared
store, places each checkout's `target/` under a disk budget, and collects old
outputs on its own. Plain `cargo` still works for contributors who do not use
mise or mbx.

- Give each worktree and each parallel agent its own target directory. A
  worktree's own `target/` is enough, and mbx manages it; a second build in
  the same checkout uses `CARGO_TARGET_DIR=target/<name>`, which stays inside
  the managed target. Never point builds at a shared target directory: Cargo's
  lock serializes them, one worktree's test run can execute another's binary,
  and the store already shares compiled outputs.
- Never vary `codegen-units` or other compiler flags per agent. Each variant
  is a separate cache entry and fills the disk.
- Do not `cargo clean` to save space. `mbx gc --dry-run` previews collection
  and `mbx gc` runs it now; `mbx cache stats` shows what is held.
- When a build is colder than expected, `mbx explain --last` says what missed
  the cache and why.
- Never put build output or large scratch files in `/tmp`. It is a small
  in-memory filesystem with a per-user quota, and filling it breaks every
  shell on the machine.
- Delete one-off QA, packaging, and release-validation directories (under
  `.cache/` or `~/.cache/`) once their result is recorded.

## Definition of done

- Add focused regression tests for changed behaviour. Use the `demo` feature
  for deterministic UI coverage and screenshots.
- Update the README and docs when user-visible behaviour, settings, files, or
  network access changes.
- Run the full checks from `CONTRIBUTING.md`. Do not weaken a lint, delete a
  test, or add an `allow` merely to make CI green without explaining why the
  underlying rule does not apply.
- Report platform coverage honestly. Do not claim a platform was tested when
  it was only compiled or reasoned about.

## Releases

Melba publishes paired, publisher-signed Linux executable archives. Follow the
repository-root [update and publication procedure](../../docs/updates.md) and
[signed release workflow](../../.github/workflows/release-publish.yml), not a
manual tag-and-release shortcut. [Publication scope](../../PUBLICATION.md)
separates the source-export workflow from binary distribution.

- Publication requires explicit authorization for the current request. Use the
  existing workflow on `main` after the release PR's exact-head checks pass;
  preserve the publisher key and historical releases.
- Update both application manifests and their lockfiles together. Leave
  independently versioned support crates alone unless their own version changes.
- Write user-facing notes under repository-root `docs/releases/`, matching the
  recent stable releases. State verification limits honestly; do not infer live
  sign-in, physical audio or platform acceptance from synthetic tests or builds.
- Only advance published-install documentation and Nix hashes after verifying
  the public archives, signature and installer results. Changing source version
  metadata does not establish that a release exists.
