# Melba 0.4.0 release gate

## Next Agent Prompt

The maintainer authorized the 0.4.0 preparation and release sequence after accepting the installed responsive-selection and settings-layout builds. The release branch includes both accepted commits. Prepare one release PR, verify its exact head, and do not publish a tag or release until the gates below are satisfied. Use the existing signed release workflow on main; retain the established publisher key and all historical releases. Never direct-push main.

## Scope and invariants

- Ship direct upcoming-occurrence selection, cancellable directional Next/Previous loading, and the accepted responsive Settings layout. No additional DSP, alternate sources, account features or redesign.
- Keep user settings, account and queue data readable across the official 0.3.30 upgrade. No migration or compatibility scaffolding is needed unless an actual failure demonstrates it.
- Both application manifests and lockfiles say 0.4.0. Independently versioned support crates do not change merely to match an application release.
- Local tests use bounded single-job builds and private HOME/XDG/D-Bus with ALSA null. Never sign out the normal session or take its audio device.
- Fake peers prove protocol/failure handling, not fresh Apple sign-in or server-side expiry. A copied valid token proves restore only, never new sign-in. Keep credentials and account evidence private, sanitized and outside the repository.
- The release version is not the published version. Published-install docs and Nix remain pinned to the actual signed release until new assets have been verified.

## Gates

- [ ] Version metadata, candidate notes and local package/upgrade checks.
- [ ] Matching-source workspace and default/all-features GUI verification.
- [x] Authentication scope decision (2026-11-16): the maintainer authorized release without repeating fresh sign-in, real-session expiry recovery or GUI sign-out/re-login. The authentication core and GUI sign-in/restore/sign-out bodies match v0.3.30; shared HTTP cancellation and test-backend D-Bus isolation were reviewed separately. This is accepted residual risk, not live-account test acceptance. Offline alpha.2 checks cover synthetic credential storage/rejection/clear and the existing seven password-input regressions only.
- [x] Playback scope decision (2026-11-16): the maintainer reported stable playback on alpha.2 and authorized the stable release on that basis. A new isolated full-catalog AAC measurement beyond 30 seconds is not claimed or required for this release. Keep that broader account/playback acceptance outstanding in the roadmap.
- [ ] Release PR and current-head CI; accepted visual scope is unchanged from the maintainer's installed review.
- [ ] Merge only after release-critical checks, then use the existing publisher-signed workflow for v0.4.0.
- [ ] Independently verify published archives, checksum signature and installer, then update release docs and Nix hashes in a follow-up PR.

## Evidence ownership

Source and tests own behavior; existing packaging and publication scripts own archive, signature and installation policy. Reuse them rather than creating a parallel installer. Private run receipts are supplementary evidence, not public release acceptance. Current publication truth is GitHub Releases, not this plan.
