# Contributing to Melba

Melba is a native Apple Music client for Linux. This crate is the desktop
app; the `melbad` playback daemon lives at the repository root. Changes
should improve the app without adding a browser engine, fallback audio
services, or another backend beside `melbad`.

## Before opening an issue

Search open and closed issues first. For a bug, include the log and the
exact steps to reproduce it. Reports without enough information to
investigate may be closed.

For a feature, explain the user problem. Discuss large changes in an issue
before writing code. Existing code does not guarantee that a feature fits
the project.

Some boundaries are deliberate:

- Playback requires an Apple Music subscription. `melbad` decodes AAC-256
  through the Widevine CDM already installed on the machine, and ALAC
  lossless through a decrypt service the user supplies and configures
  (see the root README, "Subscription playback"). Proposals that bypass
  Apple Music's DRM or impersonate another client are out of scope.
- Apple Music tracks must come from Apple Music. Substituting audio from
  YouTube, Piped, `yt-dlp`, or another catalogue is out of scope.
- Melba will not embed a browser engine, add telemetry, or introduce a
  Melba-operated service.

Apple's MusicKit and `/v1/me` library API define what a client can do;
playlist rename, reorder, and cover upload are not available to web-origin
tokens, for example, and the app surfaces that honestly rather than
silently ignoring the action.

Duplicate, out-of-scope, or incomplete issues may be closed with a short
explanation.

A bug can be closed once its fix is on `master` and the relevant checks
pass, with the commit and release status stated. Reporter confirmation is
welcome but is not required for closure. Reopen the issue if it persists
after updating.

## Design principles

1. **Native and fast.** Startup time, idle work, memory use, and binary size
   are product features. Keep the UI thread free of network and disk waits.
2. **Focused.** Prefer a complete, coherent workflow over a collection of
   settings, modes, and speculative features.
3. **Honest integrations.** Use Apple's MusicKit endpoints and the `melbad`
   daemon for what they support. Do not scrape, impersonate capabilities,
   bypass technical protections, or silently replace one service with
   another.
4. **Small dependency surface.** Reuse the standard library and existing
   crates where practical. A new dependency needs a concrete benefit worth
   its build time, binary size, maintenance, and security cost.
5. **Visible failure, private data.** Errors should be actionable, rate
   limits should be respected, and credentials must never appear in logs.
   Network behaviour belongs in the documentation.

Linux is the tested platform today. The code still carries macOS and
Windows paths behind `cfg` blocks; keep them compiling even though they
are not part of the verified surface.

## Pull requests

Keep each pull request to one change. Explain why it belongs in Melba,
what changed, and how you tested it. Avoid unrelated formatting, refactors,
generated prose, and large mechanical rewrites.

`master` has a linear history. Outside pull requests are squash-merged
into one focused commit with contributor credit; merge commits are not
accepted. Maintainer work is committed directly on `master`, one topic per
commit. Use fast-forward-only pulls and rebase unpublished local commits
when needed. Do not rewrite published history without explicit maintainer
approval.

The same rules apply to hand-written and AI-assisted changes. The author
must understand every line and answer review comments with specific
reasoning.

Code changes should include tests for behaviour that can regress. UI
changes should include before/after screenshots or a short recording in
the pull request, not in the repository, and should use demo mode where
possible. User-visible behaviour, settings, files, or network access must
be documented in the same pull request.

Dependencies come from crates.io or from repositories the maintainer owns
under `github.com/crmne/` (the `egui`, `winit`, and `fastframe` forks the
app already builds on). A `git` dependency or `[patch]` entry may point
only at such a repository, and adding or changing one needs the
maintainer's explicit approval. Never point a dependency at your own fork
or any other source. If a dependency needs a fix, send it upstream and
link it from the pull request; the maintainer can carry it in a `crmne`
fork until a release includes it.

### Settings

Do not add a setting (a new field in `Settings`, a toggle, a slider, a
choice of options) without the maintainer's approval in the issue first.
The answer is usually no: prefer a good default that serves everyone, or a
keyboard shortcut or command for the people who want something else. A
pull request that adds a setting without that approval will be asked to
remove it.

### Visual reviews

Review evidence lives in the pull request, never in the repository. Attach
captures and recordings to the pull request description, or to a comment
when you update them, by dragging the files into GitHub's editor. Do not
commit screenshots, comparison pages or recordings, under `docs/assets/`
or anywhere else. Only images the project itself uses, such as an icon the
app draws, belong in the repository.

Use actual before-and-after captures, from demo mode where possible. Match
the data, page, interaction state, zoom and window size so the difference
shows the proposed change. Identify the baseline and candidate, and report
which platforms actually produced the captures.

Lay the captures out in the description as a table with Before and After
side by side, for light and dark themes at a narrow and a normal window
size. Show relevant open menus, loading, error and empty states, and
attach a short recording when the change moves. Explain the visible change
briefly and list any remaining checks.

An interactive HTML comparison, with theme and size selectors and Before
and After buttons or a slider, helps with larger changes. Attach it to the
pull request as one self-contained `.html` file with the captures
embedded, which GitHub hands readers as a download to open in a browser,
or publish it elsewhere, for example with GitHub Pages from your fork, and
link it from the description. A gist does not work: GitHub shows its HTML
as source.

When reviewing several changes, provide one index with the PR number and
name, a selector and Previous/Next controls, and a direct link to each
comparison. Load only the selected review, and publish the index outside
the repository too. The maintainer can approve by PR number and give
exceptions or requested adjustments in a normal message.

Visual approval covers the described appearance and interaction;
integration still requires the relevant checks. Retain approval across
rebases that preserve that scope. Implement explicitly requested
adjustments and update the evidence; ask again only if the resulting scope
goes beyond what was approved or requested.

### Checks

Run the same checks CI runs before submitting:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

`--all-features` builds the `demo` feature, which carries much of the UI
coverage; keep it green too.

For daemon-side or end-to-end behaviour, the repository-level gate is

```sh
MELBA_BIN=./target/debug/melbad ./ci/check.sh
```

which fail-closes on the real contract gates (imports, scan, transport,
licenses, MPRIS serve/control) and honestly skips what the host lacks
(hardware probe, session bus, the optional `gx` tools).

Translation changes: the catalogs under `assets/i18n/` are compiled at
build time; `cargo test --test localization` asserts every catalog covers
the template and keeps named placeholders. When translatable source
strings change, regenerate `assets/i18n/melba.pot` with GNU gettext tools
(`xgettext` with Rust support) and refresh the `.po` files, reviewing any
fuzzy or missing entries. Normal Cargo builds do not need gettext tools.

Credential-handling changes must never log tokens or proxy passwords;
the settings and HTTP modules carry tests for that (`settings.rs`,
`http.rs`). The `music-user-token` itself is captured and stored by
`melbad`, not by this crate.

When changing `Cargo.lock` or `flake.nix`, also verify `nix build
.#default` on a Nix host. The flake exists but is currently unverified on
the dev host, so say so in the pull request when you cannot run it.

By contributing, you agree that your contribution is licensed under the
project's MIT License.
