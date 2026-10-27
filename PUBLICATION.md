# Publication scope

This document describes the boundaries for **source publication, source-only export and separately published signed releases**. It is not legal clearance and does not by itself authorize redistribution or a new release.

As of **2026-10-27**, the public [v0.3.7 release](https://github.com/nekomario28/melba/releases/tag/v0.3.7) contains signed x86_64 and aarch64 binary assets and their checksum/signature files. Therefore “source-only export” below refers to one *export workflow*, **not** a claim that the project publishes no binaries. The release process has separate signing, notice and packaging checks (see [Updates](docs/updates.md) and [NOTICE.md](NOTICE.md)).

## Included in a source publication

- Application, daemon and helper sources with their distinct root and desktop-crate licenses, plus retained third-party notices and MPL source availability.
- Tests, fixtures and evidence whose provenance is recorded in [materials/README.md](materials/README.md); ALAC fixtures come from [materials/recipes/alac-fixtures.sh](materials/recipes/alac-fixtures.sh).
- Synthetic illustrations and generated documentation, subject to the applicable content review.

## Excluded from a source-only export or normal Melba distribution

- Binary packages and CI artifacts are **not copied into a source-only export**. Separately generated, signed Melba executable archives may be published as release assets when their gates pass.
- Wrapper runtime bundles, rootfs images, APKs/XAPKs and Apple/Android shared libraries are not included in the source archive or Melba packages.
- DRM keys or license responses, decrypted media, signed URLs, tokens, account databases or other user state.
- Real artwork, lyrics and screenshots without a separate rights review.
- Nonempty extracted-string outputs under `evidence/native-scan/` (URLs, certificate text, SQL/JNI fragments) are withheld from the public export and retained privately. Empty extraction outputs may remain as evidence that the scan found no matching strings.

The explicit staging script used by wrapper setup is source code, not a bundled runtime. It is not run by the normal Melba installer or at app startup. The separate [melba-wrapper-bootstrap](https://github.com/nekomario28/melba-wrapper-bootstrap) project can stage an upstream service locally when the user explicitly runs its script. Staging does not resolve the service's license or Apple's terms; ALAC also works with an already installed compatible service or `wrapper_url`. See [Wrapper control](docs/wrapper.md).

Documentation of boundaries and service control is not permission to obtain, run or redistribute third-party components.

## Claims and verification

Capability statements are limited to what was measured on the specified track, codec, sink and installation path. A valid ALAC decode, source label or ALSA verdict does not establish Apple affiliation, authorization or the same output on all hardware. See [Audio and playback](docs/audio.md), [Roadmap](docs/roadmap.md), [materials/README.md](materials/README.md) and [NOTICE.md](NOTICE.md).

A source export or successful signed-binary build does not by itself establish image rights, runtime licensing, signing-key ownership, device-level acceptance or permission to publish a different artifact.

## Generating a source-only export

Run `python3 ci/export-public.py DEST` from a clean worktree. It reads `HEAD` only, withholds nonempty extracted-string outputs, records their identities in `public_exclusions`, rewrites repository metadata to the clean `melba` name, and leaves the result uncommitted for review. It does not push, upload releases or change visibility.

## Separate release decisions

Visibility changes, pushes to another remote, release uploads, binary signing and history rewriting each need explicit decisions. Whether a separately published export uses the full history or a current-tree-only snapshot is an independent choice; old paths, images, binaries and attribution commits may still exist in Git history.
