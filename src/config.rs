//! `~/.config/melba/config.json` — optional local configuration (wrapper
//! service URL/mode, CDM path). A leaf module: anything may read config,
//! config reads nothing back — that is what keeps the stream→wrapper
//! direction acyclic.
use std::path::PathBuf;

/// Where the wrapper for ALAC comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapperMode {
    /// `melbad` runs the decrypt service itself.
    Managed,
    /// Anything else, including the default: an external decrypt service at
    /// `wrapper_url`. `serde(other)` must be the last variant, and it keeps a
    /// value this build does not know from failing the whole file.
    #[default]
    #[serde(other)]
    External,
}

/// Which stream to ask for.
///
/// `Auto` is what the player has always done: lossless when a decrypt service
/// is configured and the track carries a lossless variant, AAC-256 otherwise.
/// `Aac` exists for people who would rather not run the service, and `Lossless`
/// says the preference out loud so a fallback is a visible event rather than a
/// silent downgrade.
///
/// Distinguishing a hi-res ALAC rendition from a lossless one uses the master's
/// per-variant `BANDWIDTH` / `AVERAGE-BANDWIDTH` (`melba-cdm::pick_alac_variant`).
/// A live account's master has not been read against this choice yet, so treat
/// the `HiRes` setting as untested against the service, not as a measured result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioQuality {
    Aac,
    Lossless,
    /// The highest-bitrate ALAC rendition the master offers.
    HiRes,
    /// Anything else, including the default. `serde(other)` has to be the last
    /// variant and keeps a value this build does not know from failing the file.
    #[default]
    #[serde(other)]
    Auto,
}

/// `config.json` as this build understands it.
///
/// The file used to be read as free-form JSON, so every caller indexed it and
/// re-checked the shape of each value. Reading it as this struct puts the
/// shape in one place; keys this build does not know about are kept in `extra`
/// and written back, so a newer Melba's settings survive an older one saving
/// the file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapper_url: Option<String>,
    /// Allow a wrapper that is not on the loopback interface.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrapper_allow_remote: bool,
    /// `auto`, `default`, or `hw:X,Y`; validated on read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_device: Option<String>,
    /// Which stream to ask the service for. See [`AudioQuality`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_quality: Option<AudioQuality>,
    /// Widevine CDM shared object to prefer over the browser installs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdm_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapper_mode: Option<WrapperMode>,
    /// Settings written by another build, preserved verbatim.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Config {
    /// Read the file. A missing, unreadable or malformed file is the default
    /// configuration: this is read on playback paths that must not fail.
    pub fn load() -> Self {
        config_dir()
            .and_then(|dir| std::fs::read(dir.join("config.json")).ok())
            .and_then(|bytes| Self::parse(&bytes).ok())
            .unwrap_or_default()
    }

    /// Parse a file body. Separate from [`Config::load`] so the shape is
    /// testable without a filesystem.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// Write this configuration atomically, creating the directory.
    pub fn save(&self) -> Result<(), String> {
        let dir = config_dir().ok_or("no XDG_CONFIG_HOME or HOME")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join(".config.json.tmp");
        std::fs::write(&tmp, &body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("config.json")).map_err(|e| e.to_string())
    }

    /// Which stream the player should ask for.
    pub fn audio_quality(&self) -> AudioQuality {
        self.audio_quality.unwrap_or_default()
    }

    /// The audio device to open, if the file names a valid one.
    pub fn audio_device(&self) -> Option<String> {
        self.audio_device
            .as_deref()
            .filter(|value| valid_audio_device(value))
            .map(|value| value.trim().to_string())
    }

    /// This file's own opt-in for a non-loopback wrapper. Reading it here
    /// keeps one owner for the file's shape; see
    /// [`Config::allows_remote_wrapper`] for the combined answer.
    pub fn allows_remote_wrapper(&self) -> bool {
        self.wrapper_allow_remote
    }
}

/// The configuration in effect. Every read goes through the typed struct.
pub fn config() -> Config {
    Config::load()
}

fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|p| p.join("melba"))
}

/// `env` XDG base dir or `$HOME/<under_home>` — `None` when neither is set.
/// Every module resolves user dirs through this one leaf helper: a homeless
/// process gets a clean "no HOME" failure or a skipped cache instead of a
/// panic, or worse, a `.config` written relative to whatever cwd it ran in.
///
/// Under `cfg(test)` an unset variable resolves inside a per-process
/// temporary root instead of `$HOME`. A test that drives the real code path —
/// `melbad`'s queue persistence is reached by `advance()` — used to overwrite
/// the live `~/.local/state/melba/queue.json` with its synthetic entries.
/// That destroyed the user's queue twice: once with `["a","b"]` and again
/// with the demo fixtures. Removing the offending test each time did not stop
/// the class; this makes the write impossible by construction. A test that
/// wants a real directory still sets the variable explicitly.
pub fn xdg_dir(env: &str, under_home: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(env)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Some(dir);
    }
    if cfg!(test) {
        return Some(test_root().join(env));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(under_home))
}

/// The temporary root a test's XDG directories resolve under.
///
/// Keyed by thread: the harness gives each test its own thread, so two tests
/// that both persist state never share a file, while one test sees a stable
/// path across every call. A process-wide root would make them collide.
#[cfg(test)]
fn test_root() -> PathBuf {
    let key = format!("{:?}", std::thread::current().id())
        .replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let root = std::env::temp_dir().join(format!("melba-test-xdg-{}-{key}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    root
}

#[cfg(not(test))]
fn test_root() -> PathBuf {
    unreachable!("only used under cfg(test)")
}

/// User-supplied Android-native wrapper service URL, when configured
/// and accepted. Loopback `http`/`https`/`tcp` (and bare `host:port`)
/// only, unless `wrapper_allow_remote` is true in this file or
/// `MELBA_WRAPPER_ALLOW_REMOTE=1`. Other schemes are never accepted.
/// The upstream wrapper's key and decrypt ports are unauthenticated;
/// see `WrapperKeySource` in melba-cdm.
pub fn wrapper_url() -> Option<String> {
    let raw = config().wrapper_url?;
    melba_cdm::stream::vetted_wrapper_url(&raw).ok()
}

/// Error for a `wrapper_url` that is set but rejected by the loopback check.
pub fn wrapper_url_rejection() -> Option<String> {
    let raw = config().wrapper_url?;
    if raw.trim().is_empty() {
        return None;
    }
    melba_cdm::stream::vetted_wrapper_url(&raw).err()
}

/// Persist `wrapper_url` in config.json (atomically, other keys kept).
/// `None`/empty removes the key — ALAC off, AAC only.
pub fn set_wrapper(url: Option<&str>) -> Result<(), String> {
    let mut cfg = config();
    cfg.wrapper_url = match url.map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => Some(melba_cdm::stream::vetted_wrapper_url(u)?),
        None => None,
    };
    cfg.save()
}

/// Output device override: `auto`, `default` (OS mixer) or `hw:X,Y`.
/// Unknown values are ignored rather than trusted as a device path.
pub fn valid_audio_device(value: &str) -> bool {
    let v = value.trim();
    if v == "auto" || v == "default" {
        return true;
    }
    let Some(rest) = v.strip_prefix("hw:") else {
        return false;
    };
    let Some((card, dev)) = rest.split_once(',') else {
        return false;
    };
    !card.is_empty()
        && !dev.is_empty()
        && card.bytes().all(|b| b.is_ascii_digit())
        && dev.bytes().all(|b| b.is_ascii_digit())
}

/// The configured quality preference.
pub fn audio_quality() -> AudioQuality {
    config().audio_quality()
}

/// Persist the quality preference; `auto` removes the key.
pub fn set_audio_quality(quality: AudioQuality) -> Result<(), String> {
    let mut cfg = config();
    cfg.audio_quality = match quality {
        AudioQuality::Auto => None,
        other => Some(other),
    };
    cfg.save()
}

pub fn audio_device() -> Option<String> {
    config().audio_device()
}

pub fn set_audio_device(value: &str) -> Result<(), String> {
    if !valid_audio_device(value) {
        return Err("audio_device must be auto, default, or hw:X,Y".into());
    }
    let mut cfg = config();
    cfg.audio_device = Some(value.trim().to_string());
    cfg.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's real state must be unreachable from a test process when
    /// the XDG variable is unset. The fallback cannot be observed apart from
    /// the environment, so this asserts its shape: the resolution lands under
    /// this process's temporary root and never under `$HOME`.
    ///
    /// A harness that sets the variable explicitly opts back in (that is the
    /// designed escape hatch), so the assertion follows the environment
    /// instead of assuming it.
    #[test]
    fn tests_never_resolve_a_user_directory_by_default() {
        let home = std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_default();
        let configured = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        let resolved = xdg_dir("XDG_STATE_HOME", ".local/state").expect("a state dir");
        match configured {
            Some(explicit) => assert_eq!(resolved, explicit, "an explicit value must win"),
            None => {
                assert!(resolved.starts_with(std::env::temp_dir()), "{resolved:?}");
                assert!(
                    !resolved.starts_with(&home),
                    "a test reached $HOME: {resolved:?}"
                );
                let queue = crate::player::queue_state_path().expect("a queue path");
                assert!(queue.starts_with(std::env::temp_dir()), "{queue:?}");
                assert!(!queue.starts_with(&home), "{queue:?}");
            }
        }
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("MELBA_WRAPPER_ALLOW_REMOTE", v),
                None => std::env::remove_var("MELBA_WRAPPER_ALLOW_REMOTE"),
            }
        }
    }

    /// Unknown keys belong to another build; saving this one must not drop
    /// them. A settings file is shared state, not this build's private one.
    #[test]
    fn a_save_keeps_settings_this_build_does_not_know() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-extra-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::fs::create_dir_all(dir.join("melba")).unwrap();
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"audio_device":"hw:1,0","crossfade_seconds":6,"future":{"nested":true}}"#,
        )
        .unwrap();
        set_wrapper(Some("tcp://127.0.0.1:10020")).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("melba/config.json")).unwrap()).unwrap();
        assert_eq!(saved["audio_device"], "hw:1,0");
        assert_eq!(saved["crossfade_seconds"], 6);
        assert_eq!(saved["future"]["nested"], true);
        assert_eq!(saved["wrapper_url"], "tcp://127.0.0.1:10020");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every key this build reads has a type, so a value of the wrong shape is
    /// one ignored setting rather than a whole file that fails to parse.
    #[test]
    fn the_known_keys_are_typed() {
        let cfg = Config::parse(
            br#"{"wrapper_url":"tcp://127.0.0.1:10020","wrapper_allow_remote":true,
                 "audio_device":"hw:0,0","cdm_path":"/opt/cdm/libwidevinecdm.so",
                 "wrapper_mode":"managed","unknown_key":7}"#,
        )
        .expect("parses");
        assert_eq!(cfg.wrapper_url.as_deref(), Some("tcp://127.0.0.1:10020"));
        assert!(cfg.allows_remote_wrapper());
        assert_eq!(cfg.audio_device(), Some("hw:0,0".to_string()));
        assert_eq!(cfg.cdm_path.as_deref(), Some("/opt/cdm/libwidevinecdm.so"));
        assert_eq!(cfg.wrapper_mode, Some(WrapperMode::Managed));
        assert_eq!(cfg.extra["unknown_key"], 7);
    }

    /// A wrapper mode this build does not recognise is the external one, not a
    /// parse failure that would silently drop the rest of the file.
    #[test]
    fn an_unknown_wrapper_mode_reads_as_external() {
        let cfg = Config::parse(br#"{"wrapper_mode":"sidecar","audio_device":"default"}"#)
            .expect("parses");
        assert_eq!(cfg.wrapper_mode, Some(WrapperMode::External));
        assert_eq!(cfg.audio_device(), Some("default".to_string()));
    }

    /// The quality preference is one of four words, and an unknown value in
    /// the file reads as `auto` rather than failing the file.
    #[test]
    fn the_quality_preference_round_trips() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-quality-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        assert_eq!(audio_quality(), AudioQuality::Auto, "unset means auto");
        for (quality, word) in [
            (AudioQuality::Aac, "aac"),
            (AudioQuality::Lossless, "lossless"),
            (AudioQuality::HiRes, "hi-res"),
            (AudioQuality::Auto, "auto"),
        ] {
            set_audio_quality(quality).unwrap();
            assert_eq!(audio_quality(), quality, "{word}");
        }
        // `auto` removes the key rather than writing a value that means default
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("melba/config.json")).unwrap()).unwrap();
        assert!(saved.get("audio_quality").is_none(), "{saved}");
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"audio_quality":"hires"}"#,
        )
        .unwrap();
        assert_eq!(
            audio_quality(),
            AudioQuality::Auto,
            "a value this build does not know is auto, not a parse failure"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A malformed file is the default configuration, and writing over it is
    /// still atomic and readable.
    #[test]
    fn a_malformed_file_reads_as_defaults() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-broken-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::fs::create_dir_all(dir.join("melba")).unwrap();
        std::fs::write(dir.join("melba/config.json"), b"{ not json").unwrap();
        assert_eq!(config().wrapper_url, None);
        set_audio_device("hw:0,0").unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("melba/config.json")).unwrap()).unwrap();
        assert_eq!(saved["audio_device"], "hw:0,0");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrapper_config_roundtrip() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        assert_eq!(wrapper_url(), None);
        set_wrapper(Some(" tcp://127.0.0.1:10020 ")).unwrap();
        assert_eq!(wrapper_url().as_deref(), Some("tcp://127.0.0.1:10020"));
        // survives a second write preserving nothing else needed
        set_wrapper(None).unwrap();
        assert_eq!(wrapper_url(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn audio_device_roundtrip_and_validation() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-audio-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        assert_eq!(audio_device(), None);
        set_audio_device("hw:3,3").unwrap();
        assert_eq!(audio_device().as_deref(), Some("hw:3,3"));
        assert!(set_audio_device("default").is_ok());
        assert_eq!(audio_device().as_deref(), Some("default"));
        for bad in ["hw:x,3", "hw:3", "file:///x", "hw:3,3,4"] {
            assert!(set_audio_device(bad).is_err(), "{bad}");
        }
        // An invalid file value is ignored, not returned as a device.
        std::fs::create_dir_all(dir.join("melba")).unwrap();
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"audio_device":"hw:../../x"}"#,
        )
        .unwrap();
        assert_eq!(audio_device(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrapper_url_rejects_remote_without_opt_in() {
        let _g = lock();
        let dir = std::env::temp_dir().join(format!("melbacfg-remote-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let prev = std::env::var_os("MELBA_WRAPPER_ALLOW_REMOTE");
        std::env::remove_var("MELBA_WRAPPER_ALLOW_REMOTE");
        let _restore = RestoreEnv(prev);
        assert!(set_wrapper(Some("http://evil.example/key")).is_err());
        std::fs::create_dir_all(dir.join("melba")).unwrap();
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"wrapper_url":"http://evil.example/key"}"#,
        )
        .unwrap();
        assert_eq!(wrapper_url(), None);
        assert!(wrapper_url_rejection().is_some());
        std::env::set_var("MELBA_WRAPPER_ALLOW_REMOTE", "1");
        assert_eq!(wrapper_url().as_deref(), Some("http://evil.example/key"));
        std::env::remove_var("MELBA_WRAPPER_ALLOW_REMOTE");
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"wrapper_url":"http://evil.example/key","wrapper_allow_remote":true}"#,
        )
        .unwrap();
        assert_eq!(wrapper_url().as_deref(), Some("http://evil.example/key"));
        std::fs::write(
            dir.join("melba/config.json"),
            r#"{"wrapper_allow_remote":true}"#,
        )
        .unwrap();
        assert!(set_wrapper(Some("file:///tmp/x")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
