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
    legacy_names: &[],
    ..UpdateConfig::new(
        "nekomario28/melba",
        "Melba",
        "melba",
        env!("CARGO_PKG_VERSION"),
    )
};

/// An updater on melba's HTTP client, through the configured proxy.
pub fn updater(proxy: &crate::settings::ProxyConfig) -> anyhow::Result<Updater> {
    let builder = crate::http::blocking_builder(proxy).map_err(anyhow::Error::msg)?;
    Ok(Updater::new(CONFIG, ReqwestTransport::new(builder)?))
}

/// fastframe replaces the GUI executable, not its co-installed daemon.
/// Until paired-package rollback exists, never leave the frontend ahead of
/// melbad. A package manager/reinstall can update both; a staged GUI update is
/// safe only after the matching daemon has already been installed.
#[cfg(target_os = "linux")]
pub(crate) fn require_matching_daemon_version(version: &str) -> anyhow::Result<()> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    const INSTRUCTION: &str = "Install Melba and melbad together from the release package. This in-app updater cannot replace a different-version playback daemon.";
    let mut command = crate::daemon_worker::daemon_command(std::env::current_exe().ok());
    let mut child = command
        .arg("--version")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| anyhow::anyhow!(INSTRUCTION))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(INSTRUCTION);
            }
        }
    };
    let mut output = String::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(512).read_to_string(&mut output);
    }
    anyhow::ensure!(
        status.success() && daemon_version_matches(&output, version),
        INSTRUCTION
    );
    Ok(())
}

#[cfg(target_os = "linux")]
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
    fn update_config_is_valid() {
        CONFIG.validate().unwrap();
        assert_eq!(CONFIG.current_version, env!("CARGO_PKG_VERSION"));
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
