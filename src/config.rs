//! `~/.config/melba/config.json` — optional local configuration (wrapper
//! service URL/mode, CDM path). A leaf module: anything may read config,
//! config reads nothing back — that is what keeps the stream→wrapper
//! direction acyclic.
use std::path::PathBuf;

pub fn config() -> serde_json::Value {
    config_dir()
        .and_then(|d| std::fs::read(d.join("config.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|p| p.join("melba"))
}

/// `env` XDG base dir or `$HOME/<under_home>` — `None` when neither is set.
/// Every module resolves user dirs through this one leaf helper: a homeless
/// process gets a clean "no HOME" failure or a skipped cache instead of a
/// panic, or worse, a `.config` written relative to whatever cwd it ran in.
pub fn xdg_dir(env: &str, under_home: &str) -> Option<PathBuf> {
    std::env::var_os(env)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(under_home)))
}

/// User-supplied Android-native wrapper service URL, when configured
/// and accepted. Loopback `http`/`https`/`tcp` (and bare `host:port`)
/// only, unless `wrapper_allow_remote` is true in this file or
/// `MELBA_WRAPPER_ALLOW_REMOTE=1`. Other schemes are never accepted.
/// The upstream wrapper's key and decrypt ports are unauthenticated;
/// see `WrapperKeySource` in melba-cdm.
pub fn wrapper_url() -> Option<String> {
    let cfg = config();
    let raw = cfg["wrapper_url"].as_str()?;
    melba_cdm::stream::vetted_wrapper_url(raw).ok()
}

/// Error for a `wrapper_url` that is set but rejected by the loopback check.
pub fn wrapper_url_rejection() -> Option<String> {
    let cfg = config();
    let raw = cfg["wrapper_url"].as_str()?;
    if raw.trim().is_empty() {
        return None;
    }
    melba_cdm::stream::vetted_wrapper_url(raw).err()
}

fn write_config(c: &serde_json::Value) -> Result<(), String> {
    let dir = config_dir().ok_or("no XDG_CONFIG_HOME or HOME")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let body = serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?;
    let tmp = dir.join(".config.json.tmp");
    std::fs::write(&tmp, &body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("config.json")).map_err(|e| e.to_string())
}

/// Persist `wrapper_url` in config.json (atomically, other keys kept).
/// `None`/empty removes the key — ALAC off, AAC only.
pub fn set_wrapper(url: Option<&str>) -> Result<(), String> {
    let mut c = config();
    match url.map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => {
            let u = melba_cdm::stream::vetted_wrapper_url(u)?;
            c["wrapper_url"] = serde_json::json!(u);
        }
        None => {
            if let Some(m) = c.as_object_mut() {
                m.remove("wrapper_url");
            }
        }
    }
    write_config(&c)
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

pub fn audio_device() -> Option<String> {
    config()["audio_device"]
        .as_str()
        .filter(|v| valid_audio_device(v))
        .map(|v| v.trim().to_string())
}

pub fn set_audio_device(value: &str) -> Result<(), String> {
    if !valid_audio_device(value) {
        return Err("audio_device must be auto, default, or hw:X,Y".into());
    }
    let mut c = config();
    c["audio_device"] = serde_json::json!(value.trim());
    write_config(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

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
