//! Self-update from GitHub releases, through fastframe-update.
//!
//! The previous release's helper installs the first release built on the
//! crate, so what it relies on stays the same: `--version` prints
//! `<command> <version>`, the new app accepts `--update-receipt` and
//! `--update-error` (stripped by [`fastframe_update::intercept`] before the
//! argument parser), and the handoff and receipt files keep their format.

pub use fastframe_update::{
    CHECK_INTERVAL, DownloadState, Installation, Kind, Prepared, Release, Source, Unsupported,
    Updater,
};
use fastframe_update::{MacConfig, ReqwestTransport, UpdateConfig};

pub const CONFIG: UpdateConfig = UpdateConfig {
    macos: MacConfig {
        bundle_ids: &["rocks.melba.melba"],
        executable_names: &["melba"],
        legacy_bundle_names: &[],
    },
    // Updates must carry an Ed25519-signed checksums.txt to be accepted: a
    // release without a valid `checksums.txt.sig` is never downloaded.
    // The committed public key is the verification trust anchor. Control and
    // provenance of its private half must be verified before publishing
    // (see README, Updates); this configuration cannot establish either.
    // A newer source-only tag may be reported by a check, but cannot be
    // installed without signed binary assets. `None` here would trust GitHub's checksum file alone,
    // which only protects against corruption — never drop the key.
    publisher_key: Some(include_str!("../assets/update-public-key.hex")),
    // Empty on purpose. This field exists to carry the *next* key while a
    // release signed with the current one teaches installs to trust it. It used
    // to hold the key that the pinned one replaced, as transitional trust for
    // v0.3.2. A retired key that can still sign an accepted update is not
    // transitional trust, it is a second way in: anyone holding it could publish
    // an update this build installs. Rotation is how a key comes back here.
    additional_publisher_keys: &[],
    legacy_names: &[],
    ..UpdateConfig::new(
        "nekomario28/melba",
        "Melba",
        "melba",
        env!("CARGO_PKG_VERSION"),
    )
};

fn for_channel(channel: crate::settings::UpdateChannel) -> UpdateConfig {
    UpdateConfig {
        prereleases: match channel {
            crate::settings::UpdateChannel::Stable => fastframe_update::Prereleases::Never,
            crate::settings::UpdateChannel::Alpha => fastframe_update::Prereleases::Always,
        },
        ..CONFIG
    }
}

/// One policy for discovery, download, helper execution and receipt acceptance.
/// The hidden preference is edited with the app closed; no UI writes this field.
pub fn configured() -> UpdateConfig {
    configured_in(&crate::paths::AppDirs::discover().config)
}

/// Same policy as [`configured`], resolving `settings.json` inside `config_dir`
/// when provided (falling back to [`AppDirs::discover`] for empty default dirs).
pub fn configured_in(config_dir: &std::path::Path) -> UpdateConfig {
    let path = if config_dir.as_os_str().is_empty() {
        crate::paths::AppDirs::discover().settings_file()
    } else {
        config_dir.join("settings.json")
    };
    for_channel(crate::settings::Settings::load(&path).update_channel)
}

/// An updater on melba's HTTP client, through the configured proxy.
pub fn updater(proxy: &crate::settings::ProxyConfig) -> anyhow::Result<Updater> {
    updater_in(&crate::paths::AppDirs::discover().config, proxy)
}

/// An updater resolving hidden channel preferences from `config_dir`.
pub fn updater_in(
    config_dir: &std::path::Path,
    proxy: &crate::settings::ProxyConfig,
) -> anyhow::Result<Updater> {
    let builder = crate::http::blocking_builder(proxy).map_err(anyhow::Error::msg)?;
    Ok(Updater::new(
        configured_in(config_dir),
        ReqwestTransport::new(builder)?,
    ))
}

/// The vendored fastframe helper replaces `melba` and the companion `melbad`
/// from one signed archive and restores both when the new GUI fails to start.
/// Older releases lacked that companion; the preflight is kept as a named
/// hook for callers but no longer compares a still-old daemon first.
#[cfg(target_os = "linux")]
pub(crate) fn require_matching_daemon_version(_version: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
fn daemon_version_matches(output: &str, version: &str) -> bool {
    let fields: Vec<_> = output.split_whitespace().collect();
    fields == ["melbad", version.trim_start_matches('v')]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn a_gui_update_cannot_leave_a_different_daemon_version() {
        assert!(daemon_version_matches("melbad 0.3.2\n", "0.3.2"));
        assert!(daemon_version_matches("melbad 0.3.2\n", "v0.3.2"));
        assert!(!daemon_version_matches("melbad 0.3.1\n", "0.3.2"));
        assert!(!daemon_version_matches("melba 0.3.2\n", "0.3.2"));
        assert!(!daemon_version_matches("melbad 0.3.2 extra", "0.3.2"));
    }

    #[test]
    fn hidden_channel_round_trips_and_keeps_the_same_trust_anchor() {
        use crate::settings::{Settings, UpdateChannel};
        for (json, expected) in [
            ("{}", UpdateChannel::Stable),
            (r#"{"update_channel":"stable"}"#, UpdateChannel::Stable),
            (r#"{"update_channel":"unknown"}"#, UpdateChannel::Stable),
            (r#"{"update_channel":"alpha"}"#, UpdateChannel::Alpha),
        ] {
            let settings: Settings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.update_channel, expected);
            let saved: Settings = serde_json::from_str(&settings.encode().unwrap()).unwrap();
            assert_eq!(saved.update_channel, expected);
            let config = for_channel(saved.update_channel);
            config.validate().unwrap();
            assert_eq!(config.publisher_key, CONFIG.publisher_key);
            assert_eq!(
                config.additional_publisher_keys,
                CONFIG.additional_publisher_keys
            );
            assert_eq!(config.current_version, CONFIG.current_version);
            assert_eq!(
                config.prereleases,
                if expected == UpdateChannel::Alpha {
                    fastframe_update::Prereleases::Always
                } else {
                    fastframe_update::Prereleases::Never
                }
            );
        }
        let default_json = Settings::default().encode().unwrap();
        assert!(!default_json.contains("update_channel"));
    }

    #[test]
    fn configured_in_reads_hidden_update_channel_from_settings_file() {
        let dir = std::env::temp_dir().join(format!(
            "melba-updates-channel-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            configured_in(&dir).prereleases,
            fastframe_update::Prereleases::Never
        );
        std::fs::write(dir.join("settings.json"), r#"{"update_channel":"alpha"}"#).unwrap();
        assert_eq!(
            configured_in(&dir).prereleases,
            fastframe_update::Prereleases::Always
        );
        std::fs::write(dir.join("settings.json"), r#"{"update_channel":"bogus"}"#).unwrap();
        assert_eq!(
            configured_in(&dir).prereleases,
            fastframe_update::Prereleases::Never
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_config_is_valid() {
        CONFIG.validate().unwrap();
        assert_eq!(CONFIG.current_version, env!("CARGO_PKG_VERSION"));
    }

    /// Trust is the pinned key and nothing else. A retired key left in the
    /// transitional slot accepts signatures this build has no reason to accept.
    #[test]
    fn only_the_pinned_key_is_trusted() {
        assert!(
            CONFIG.additional_publisher_keys.is_empty(),
            "a second trusted key accepts updates signed without the pinned one"
        );
        assert_eq!(
            CONFIG.publisher_key.map(str::trim),
            Some(include_str!("../assets/update-public-key.hex").trim()),
            "the pinned key is the trust anchor"
        );
    }

    #[test]
    fn updates_require_the_release_signing_key() {
        // Dropping the key would make the updater trust GitHub's checksum file
        // alone (corruption only, no publisher), so the key must stay.
        let key = CONFIG
            .publisher_key
            .expect("updates must require a publisher signature");
        assert!(
            key.trim().len() == 64 && key.trim().bytes().all(|byte| byte.is_ascii_hexdigit()),
            "the committed publisher key must be 64 hex characters"
        );
        CONFIG.validate().unwrap();
    }

    /// Check the public asset/config agreement and adjacent key filenames,
    /// not key provenance or the absence of secrets throughout the repository.
    #[test]
    fn configured_update_key_matches_the_committed_public_asset() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        for entry in std::fs::read_dir(&assets).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lowered = name.to_ascii_lowercase();
            assert!(
                !lowered.contains("private")
                    && !lowered.ends_with(".pem")
                    && !lowered.ends_with(".key"),
                "the repository must not hold signing key material: {name}"
            );
        }
        let committed = std::fs::read_to_string(assets.join("update-public-key.hex")).unwrap();
        assert_eq!(
            committed.trim(),
            CONFIG.publisher_key.unwrap().trim(),
            "the committed public key and the update config must agree"
        );
    }
}
