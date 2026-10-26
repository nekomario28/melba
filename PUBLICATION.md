# Publication scope

This repository is prepared for **source-only** publication. This file records
the boundary; it is not a legal clearance and it does not authorize a release.

## Included in a source publication

- Application, daemon and helper source under the repository's MIT/Apache-2.0
  terms, with third-party notices and MPL source availability.
- Tests, fixtures and evidence whose provenance is recorded in
  [materials/README.md](materials/README.md); ALAC fixtures come from the
  recorded recipe in `materials/recipes/alac-fixtures.sh`.
- Synthetic illustrations and generated documentation.

## Not published from this repository

- Release binaries, packages, installers-with-payloads or CI artifacts.
- Wrapper runtime bundles, rootfs images, APKs/XAPKs, and Apple/Android shared
  libraries; they are not distributed by the supported public path and must not
  be added to a source archive or package.
- DRM keys or license responses, decrypted media, signed URLs, tokens,
  account databases or other user state.
- Wrapper runtime payloads (rootfs, Apple/Android libraries, APKs) are never
  bundled. The explicit staging script used by the wrapper setup command is part of
  the source tree; it is not run by install or startup, and it stages the
  upstream bundle into the user's home only when invoked. The same pinned
  staging script is also maintained in the separate
  `melba-wrapper-bootstrap` project. ALAC playback works with an already
  installed service or a configured `wrapper_url`; neither path is legal
  clearance.
- Real cover art, lyrics, or screenshots that need their own rights review.

Documentation describes boundaries, not acquisition or decryption steps: the
operational wrapper notes are kept outside the repository. This keeps the
public source free of instructions whose contract and anti-circumvention
status is unresolved.

## Claims

Capability statements are limited to what was measured on the stated path
(for example ALAC decoding and the sink used) and must not imply Apple
affiliation, authorization, or a guarantee for every track or account.
Performance and losslessness descriptions follow
[materials/README.md](materials/README.md) and `NOTICE.md`.

## Decisions still requiring separate action

Visibility changes, pushes to another remote, release uploads, binary signing
and history rewriting each need an explicit instruction. Whether the published
form is the full history or a current-tree export remains open; old images,
paths, binaries and attribution commits still exist in history.
