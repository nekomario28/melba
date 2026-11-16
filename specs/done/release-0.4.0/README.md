# Melba 0.4.0 release rationale

## Purpose

The [stable 0.4.0 release](https://github.com/nekomario28/melba/releases/tag/v0.4.0) promotes the maintainer-accepted alpha.2 playback and Settings work without adding another runtime change. Separating promotion from feature development keeps the accepted behavior fixed while version metadata, signed archives and consumer pins advance.

The [release notes](../../../docs/releases/v0.4.0.md) describe the user-visible scope. [PR #136](https://github.com/nekomario28/melba/pull/136) passed its current-head checks before merge; the [signed publication run](https://github.com/nekomario28/melba/actions/runs/43810100465) published tag `v0.4.0` at commit `1f04f2030b2eed531ca0f4d3cc6f4c5074952f10` on 2026-11-16.

## Accepted verification scope

The maintainer reported stable playback on alpha.2 and authorized stable publication without repeating fresh sign-in, real-session expiry recovery or GUI sign-out/re-login. Those are not successful test results. The authentication core and the GUI sign-in, restore and sign-out bodies match v0.3.30. Shared HTTP cancellation and test-backend D-Bus isolation were reviewed separately rather than assumed equivalent from unchanged filenames.

Synthetic offline checks establish credential storage, invalid-input preservation, local clearing and password-input handling only. A new isolated full-catalog AAC measurement beyond 30 seconds was not performed. The broader account and first-play acceptance remains open in the [roadmap](../../../docs/roadmap.md); the decision here is to accept that residual risk for an unchanged authentication feature, not to certify those flows from existing playback.

Normal account, queue and audio-device state must remain outside diagnostic tests. Requiring the maintainer to sign out the working installation was rejected in favor of change-scoped review and explicit limits.

## Publication invariants

- Promote both application versions together, preserving independently versioned support crates and the dependency locks. Settings and queue formats remain compatible without a migration.
- Keep the established publisher key and historical releases. The [signed workflow](../../../.github/workflows/release-publish.yml) owns candidate validation, publication and published-installer checks; do not create a parallel release mechanism.
- Advance published-install documentation and [Nix pins](../../../crates/melba/nix/paired-release.nix) only after the public bytes have been verified. A source version bump is not evidence that matching assets exist.
- Distinguish ELF/ABI and container launch compatibility from desktop, real-account and physical-audio acceptance. Published archives were independently checked against the pinned key, signed manifest, architecture and paired-daemon layout without executing downloaded binaries locally.

The [transport regressions](../../../tests/transport.rs), [release-version and lock guards](../../../ci/test_release_install_tag.py), [portable packaging](../../../packaging/package-portable.sh) and publication workflow own the executable contracts. Runtime navigation rationale remains in [responsive selection](../../responsive-selection/README.md).
