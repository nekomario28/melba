mod browser_storage;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn dir() -> Option<PathBuf> {
    crate::config::xdg_dir("XDG_CONFIG_HOME", ".config").map(|p| p.join("melba"))
}

pub fn token_path() -> Option<PathBuf> {
    dir().map(|d| d.join("music-user-token"))
}

pub fn set(token: &str) -> Result<(), String> {
    store(&token_path().ok_or("no XDG_CONFIG_HOME or HOME")?, token)
}

fn valid_token(token: &str) -> Result<&str, String> {
    let token = token.trim();
    if token.is_empty() || token.len() > 16384 || token.chars().any(char::is_control) {
        return Err("invalid music-user-token".into());
    }
    Ok(token)
}

/// Automatic capture must be accepted by Apple before replacing a credential.
/// False means rejected; filesystem failures remain errors, not sign-in success.
pub(crate) fn set_verified(
    token: &str,
    validate: impl Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    store_verified(
        &token_path().ok_or("no XDG_CONFIG_HOME or HOME")?,
        token,
        validate,
    )
}

fn store_verified(
    path: &Path,
    token: &str,
    validate: impl Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    let token = valid_token(token)?;
    if validate(token).is_err() {
        return Ok(false);
    }
    store(path, token)?;
    Ok(true)
}

pub(crate) fn store(path: &Path, token: &str) -> Result<(), String> {
    let token = valid_token(token)?;
    let parent = path.parent().ok_or("token path has no parent")?;
    private_dir(parent)?;
    // Create private BEFORE writing; rename replaces a symlink, never its target.
    let tmp = parent.join(format!(".token-{}", unique_id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(token.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn get() -> Option<String> {
    std::fs::read_to_string(token_path()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn clear() -> Result<(), String> {
    match std::fs::remove_file(token_path().ok_or("no XDG_CONFIG_HOME or HOME")?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn exp_of(jwt: &str) -> Option<u64> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = b64url(payload).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v["exp"].as_u64()
}

/// minimal base64url decoder (no padding) — kept private so token storage
/// never reaches back into the API client (auth is a leaf module)
fn b64url(s: &str) -> Result<Vec<u8>, String> {
    let mut v = Vec::new();
    let mut acc = 0u32;
    let mut n = 0;
    for c in s.chars() {
        let d = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' => 62,
            '_' => 63,
            _ => return Err("b64".into()),
        };
        acc = (acc << 6) | d;
        n += 6;
        if n >= 8 {
            n -= 8;
            v.push((acc >> n) as u8);
        }
    }
    Ok(v)
}

fn unique_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .and_then(|dir| dir.set_permissions(fs::Permissions::from_mode(0o700)))
        .map_err(|e| e.to_string())
}

/// Chromium-family profile dirs that may hold a MusicKit sign-in.
/// `MELBA_AUTH_PROFILE` points at a specific profile dir (tests/override).
///
/// Covers every `Default`/`Profile *`/`Guest Profile` dir under each
/// browser root — sign-ins on a secondary profile count too — plus the
/// snap and flatpak config roots, which do not share ~/.config.
pub fn browser_profiles() -> Vec<PathBuf> {
    if let Some(p) = std::env::var_os("MELBA_AUTH_PROFILE") {
        return vec![PathBuf::from(p)];
    }
    let mut roots = vec![];
    if let Some(cfg) = crate::config::xdg_dir("XDG_CONFIG_HOME", ".config") {
        roots.extend([
            cfg.join("vivaldi"),
            cfg.join("chromium"),
            cfg.join("google-chrome"),
            cfg.join("BraveSoftware/Brave-Browser"),
            cfg.join("microsoft-edge"),
        ]);
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // containerized installs keep their config outside ~/.config
        roots.extend([
            home.join("snap/chromium/common/chromium"),
            home.join(".var/app/org.chromium.Chromium/config/chromium"),
            home.join(".var/app/com.google.Chrome/config/google-chrome"),
            home.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
            home.join(".var/app/com.vivaldi.Vivaldi/config/vivaldi"),
            home.join(".var/app/com.microsoft.Edge/config/microsoft-edge"),
        ]);
    }
    // prefer profiles that already carry browser storage; when none do
    // (a profile created but never signed in) watch every existing one —
    // the dirs appear the moment the sign-in writes them.
    let (mut found, mut bare): (Vec<PathBuf>, Vec<PathBuf>) = (vec![], vec![]);
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut profs = vec![root.join("Default")];
        if let Ok(rd) = fs::read_dir(&root) {
            profs.extend(
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("Profile ") || n == "Guest Profile")
                    }),
            );
        }
        for cand in profs {
            if !cand.is_dir() {
                continue;
            }
            if cand.join("Local Storage").is_dir() || cand.join("IndexedDB").is_dir() {
                found.push(cand);
            } else {
                bare.push(cand);
            }
        }
    }
    if found.is_empty() {
        found = bare;
    }
    found
}

/// Current Apple-origin candidates, newest first, without losing alternatives
/// when a newer-looking token is rejected by the server.
fn sync_scan(profiles: &[PathBuf]) -> Result<Vec<(String, u64)>, String> {
    let candidates = ordered_candidates(
        profiles
            .iter()
            .flat_map(|profile| browser_storage::tokens(profile)),
    );
    if candidates.is_empty() {
        Err("no MusicKit sign-in found — sign in on music.apple.com in your browser".into())
    } else {
        Ok(candidates)
    }
}

fn ordered_candidates(tokens: impl Iterator<Item = String>) -> Vec<(String, u64)> {
    let mut candidates: Vec<_> = tokens
        .map(|token| {
            let exp = exp_of(&token).unwrap_or(0);
            (token, exp)
        })
        .collect();
    candidates.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates.dedup_by(|a, b| a.0 == b.0);
    candidates
}

/// Firefox keeps Web Storage in per-origin SQLite databases
/// (`storage/default/<origin>/ls/data.sqlite`), not LevelDB — a
/// separate probe from the Chromium scan, run through the `sqlite3`
/// CLI in read-only mode so no new dependency or FFI is needed. A
/// missing binary, profile, or database is a silent skip.
fn firefox_dbs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return vec![];
    };
    let mut out = vec![];
    for root in [
        home.join(".mozilla/firefox"),
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
    ] {
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                let db = e
                    .path()
                    .join("storage/default/https+++music.apple.com/ls/data.sqlite");
                if db.is_file() {
                    out.push(db);
                }
            }
        }
    }
    out
}

/// Current candidates in one Firefox localStorage database, or an empty set.
fn firefox_tokens(db: &Path) -> Vec<(String, u64)> {
    let Ok(out) = std::process::Command::new("sqlite3")
        .args(["-readonly", "-batch"])
        .arg(db)
        .arg("SELECT hex(value) FROM data WHERE key LIKE '%user-token%'")
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    ordered_candidates(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| hex_decode(line.trim()).and_then(|bytes| find_jwt(&bytes))),
    )
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Poll the browser's localStorage until a MusicKit user token appears
/// (the user signs in in their normal browser) or `timeout` elapses.
/// On success stores it through the existing atomic 0600 path.
pub fn watch_browser(
    timeout: std::time::Duration,
    validate: impl Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    watch_profiles(
        &browser_profiles(),
        &firefox_dbs(),
        &token_path().ok_or("no XDG_CONFIG_HOME or HOME")?,
        timeout,
        validate,
    )
}

fn watch_profiles(
    profiles: &[PathBuf],
    ffdbs: &[PathBuf],
    destination: &Path,
    timeout: std::time::Duration,
    validate: impl Fn(&str) -> Result<(), String>,
) -> Result<String, String> {
    if profiles.is_empty() && ffdbs.is_empty() {
        return Err(
            "no supported browser profile found — use `melbad auth set <token>` instead".into(),
        );
    }
    let deadline = std::time::Instant::now() + timeout;
    let mut saw_expired = false;
    loop {
        let mut accept = |tok: &str, exp: u64, src: &str| -> Result<Option<String>, String> {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if exp != 0 && exp <= now {
                saw_expired = true;
                return Ok(None);
            }
            if !store_verified(destination, tok, &validate)? {
                return Ok(None);
            }
            Ok(Some(format!("token captured from {src}")))
        };
        if let Ok(candidates) = sync_scan(profiles) {
            for (tok, exp) in candidates {
                if let Some(msg) = accept(&tok, exp, "browser")? {
                    return Ok(msg);
                }
            }
        }
        for db in ffdbs {
            for (tok, exp) in firefox_tokens(db) {
                if let Some(msg) = accept(&tok, exp, "Firefox")? {
                    return Ok(msg);
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(if saw_expired {
                "only an expired MusicKit token was found — sign in again on music.apple.com".into()
            } else {
                "timed out waiting for a valid music.apple.com sign-in — sign in again in your browser".into()
            });
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// First JWT in a window — ASCII `eyJ…` or UTF-16LE `e\0y\0J\0…`.
/// Requires exactly two '.' separators (header.payload.signature).
fn find_jwt(win: &[u8]) -> Option<String> {
    fn take_jwt(bytes: &[u8]) -> Option<String> {
        let end = bytes
            .iter()
            .position(|&b| !(b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
            .unwrap_or(bytes.len());
        let s = std::str::from_utf8(&bytes[..end]).ok()?;
        (s.len() > 40 && s.matches('.').count() == 2).then(|| s.to_string())
    }
    for i in 0..win.len().saturating_sub(5) {
        if win[i..].starts_with(b"eyJ") {
            if let Some(t) = take_jwt(&win[i..]) {
                return Some(t);
            }
        }
        if win[i..].starts_with(b"e\x00y\x00J\x00") {
            let wide: Vec<u8> = win[i..].as_chunks::<2>().0.iter().map(|c| c[0]).collect();
            if let Some(t) = take_jwt(&wide) {
                return Some(t);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("melbad-auth-test-{}", unique_id()));
            private_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn token_storage_is_private_atomic_and_does_not_follow_symlinks() {
        let dir = TestDir::new();
        let path = dir.0.join("config/music-user-token");
        store(&path, "  fixture-token  ").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "fixture-token");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let victim = dir.0.join("unrelated");
        fs::write(&victim, "do not overwrite").unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        store(&path, "replacement").unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "do not overwrite");
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        for invalid in ["", " \n", "bad\nvalue"] {
            assert!(store(&path, invalid).is_err());
        }
        assert!(store(&path, &"x".repeat(16385)).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "replacement");
    }

    /// tiny base64url encoder for fixture JWTs (no padding)
    fn b64url_enc(d: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut s = String::new();
        for c in d.chunks(3) {
            let n = c.iter().fold(0u32, |a, &b| (a << 8) | b as u32) << (8 * (3 - c.len()));
            for i in 0..=c.len() {
                s.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            }
        }
        s
    }

    fn fake_jwt(exp: u64) -> String {
        format!(
            "{}.{}.QUJDQUJD",
            b64url_enc(br#"{"alg":"ES256"}"#),
            b64url_enc(format!(r#"{{"exp":{exp},"iss":"t"}}"#).as_bytes())
        )
    }

    fn storage_key(origin: &str, key: &str) -> Vec<u8> {
        format!("_{origin}\0\x01{key}").into_bytes()
    }

    fn browser_fixture(profile: &Path, edit: impl FnOnce(&mut rusty_leveldb::DB)) {
        browser_storage::fixture(&profile.join("Local Storage/leveldb"), edit);
    }

    #[test]
    fn sync_scan_rejects_tokens_from_unrelated_origins() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let jwt = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://attacker.invalid", "media-user-token"),
                format!("\x01{jwt}").as_bytes(),
            )
            .unwrap();
        });
        assert!(sync_scan(std::slice::from_ref(&profile)).is_err());
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                format!("\x01{jwt}").as_bytes(),
            )
            .unwrap();
        });
        assert_eq!(sync_scan(&[profile]).unwrap(), vec![(jwt, 9_999_999_999)]);
    }

    #[test]
    fn sync_scan_does_not_mix_records_or_revive_deleted_tokens() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let jwt = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            let key = storage_key("https://music.apple.com", "media-user-token");
            db.put(&key, format!("\x01{jwt}").as_bytes()).unwrap();
            db.compact_range(b"", b"~").unwrap();
            db.delete(&key).unwrap();
            let replaced = storage_key("https://music.apple.com", "music-user-token");
            db.put(&replaced, format!("\x01{jwt}").as_bytes()).unwrap();
            db.put(&replaced, b"\x01logged-out").unwrap();
            db.put(
                &storage_key("https://music.apple.com", "media-token"),
                format!("\x01{jwt}").as_bytes(),
            )
            .unwrap();
            db.put(
                &storage_key("https://music.apple.com", "user-token-marker"),
                b"\x01empty",
            )
            .unwrap();
        });
        assert!(sync_scan(&[profile]).is_err());
    }

    #[test]
    fn sync_scan_reads_compacted_utf16_records_and_keeps_source_unchanged() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let jwt = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            let mut value = vec![0];
            for unit in jwt.encode_utf16() {
                value.extend(unit.to_le_bytes());
            }
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                &value,
            )
            .unwrap();
            db.compact_range(b"", b"~").unwrap();
        });
        let ls = profile.join("Local Storage/leveldb");
        let contents = || {
            let mut files: Vec<_> = fs::read_dir(&ls)
                .unwrap()
                .map(|e| {
                    let p = e.unwrap().path();
                    (p.file_name().unwrap().to_os_string(), fs::read(&p).unwrap())
                })
                .collect();
            files.sort();
            files
        };
        let before = contents();
        assert!(before
            .iter()
            .any(|(n, _)| n.to_string_lossy().ends_with(".ldb")));
        assert_eq!(sync_scan(&[profile]).unwrap(), vec![(jwt, 9_999_999_999)]);
        assert_eq!(contents(), before);
    }

    #[test]
    fn sync_scan_picks_newest_exp_and_reports_empty() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let newest = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://music.apple.com", "music-user-token"),
                format!("\x01{}", fake_jwt(1_000_000)).as_bytes(),
            )
            .unwrap();
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                format!("\x01{newest}").as_bytes(),
            )
            .unwrap();
            db.put(
                &storage_key("https://music.apple.com.evil.invalid", "media-user-token"),
                format!("\x01{}", fake_jwt(99_999_999_999)).as_bytes(),
            )
            .unwrap();
        });
        assert_eq!(
            sync_scan(&[profile]).unwrap(),
            vec![(newest, 9_999_999_999), (fake_jwt(1_000_000), 1_000_000)]
        );
        assert!(sync_scan(&[dir.0.join("empty")])
            .unwrap_err()
            .contains("sign in"));
    }

    #[test]
    fn verified_capture_preserves_token_until_server_accepts_replacement() {
        let dir = TestDir::new();
        let path = dir.0.join("music-user-token");
        store(&path, "old-token").unwrap();
        assert!(!store_verified(&path, "new-token", |_| Err("rejected".into())).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "old-token");
        assert!(store_verified(&path, " new-token ", |token| {
            assert_eq!(token, "new-token");
            Ok(())
        })
        .unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new-token");
        assert!(store_verified(&path, "bad\nvalue", |_| panic!(
            "invalid token reached server"
        ))
        .is_err());
        assert!(store_verified(&dir.0, "valid-token", |_| Ok(())).is_err());
    }

    #[test]
    fn browser_watch_validates_the_apple_origin_before_replacing_a_token() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let destination = dir.0.join("music-user-token");
        let trusted = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                format!("\x01{trusted}").as_bytes(),
            )
            .unwrap();
            db.put(
                &storage_key("https://attacker.invalid", "media-user-token"),
                format!("\x01{}", fake_jwt(99_999_999_999)).as_bytes(),
            )
            .unwrap();
        });
        store(&destination, "old-token").unwrap();
        let profiles = [profile];
        assert!(watch_profiles(
            &profiles,
            &[],
            &destination,
            std::time::Duration::ZERO,
            |token| {
                assert_eq!(token, trusted);
                Err("server rejected token".into())
            }
        )
        .is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "old-token");
        watch_profiles(
            &profiles,
            &[],
            &destination,
            std::time::Duration::ZERO,
            |token| {
                assert_eq!(token, trusted);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), trusted);
    }

    #[test]
    fn browser_watch_tries_other_current_candidates_after_rejection() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        let destination = dir.0.join("token");
        let rejected = fake_jwt(99_999_999_999);
        let accepted = fake_jwt(9_999_999_999);
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://music.apple.com", "music-user-token"),
                format!("\x01{rejected}").as_bytes(),
            )
            .unwrap();
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                format!("\x01{accepted}").as_bytes(),
            )
            .unwrap();
        });
        let calls = std::cell::Cell::new(0);
        watch_profiles(
            &[profile],
            &[],
            &destination,
            std::time::Duration::ZERO,
            |token| {
                calls.set(calls.get() + 1);
                if token == accepted {
                    Ok(())
                } else {
                    Err("rejected".into())
                }
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(fs::read_to_string(destination).unwrap(), accepted);
    }

    #[test]
    fn browser_watch_does_not_submit_expired_tokens() {
        let dir = TestDir::new();
        let profile = dir.0.join("Default");
        browser_fixture(&profile, |db| {
            db.put(
                &storage_key("https://music.apple.com", "media-user-token"),
                format!("\x01{}", fake_jwt(1_000_000)).as_bytes(),
            )
            .unwrap();
        });
        let error = watch_profiles(
            &[profile],
            &[],
            &dir.0.join("token"),
            std::time::Duration::ZERO,
            |_| panic!("expired token reached the server"),
        )
        .unwrap_err();
        assert!(error.contains("expired"));
    }

    #[test]
    fn private_directories_reject_symlinks_without_chmodding_the_target() {
        let dir = TestDir::new();
        let victim = dir.0.join("victim");
        fs::create_dir(&victim).unwrap();
        fs::set_permissions(&victim, fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(private_dir(&link).is_err());
        assert_eq!(
            fs::metadata(victim).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn firefox_token_extracts_jwt_from_ls_sqlite() {
        if std::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // sqlite3 CLI absent — probe is a silent skip there too
        }
        let dir = TestDir::new();
        let db = dir.0.join("data.sqlite");
        let jwt = fake_jwt(9_999_999_999);
        let ok = std::process::Command::new("sqlite3")
            .arg(&db)
            .arg(format!(
                "CREATE TABLE data (key TEXT PRIMARY KEY, value BLOB); \
                 INSERT INTO data VALUES('media-user-token','{jwt}');"
            ))
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
        assert_eq!(firefox_tokens(&db), vec![(jwt.clone(), 9_999_999_999)]);
        assert!(firefox_tokens(&dir.0.join("missing.sqlite")).is_empty());
        assert!(std::process::Command::new("sqlite3")
            .arg(&db)
            .arg(format!(
                "INSERT INTO data VALUES('music-user-token','{}');",
                fake_jwt(99_999_999_999)
            ))
            .status()
            .unwrap()
            .success());
        let destination = dir.0.join("verified-token");
        let dbs = [db];
        assert!(
            watch_profiles(&[], &dbs, &destination, std::time::Duration::ZERO, |_| Err(
                "rejected".into()
            ))
            .is_err()
        );
        assert!(!destination.exists());
        watch_profiles(
            &[],
            &dbs,
            &destination,
            std::time::Duration::ZERO,
            |token| {
                if token == jwt {
                    Ok(())
                } else {
                    Err("rejected".into())
                }
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), jwt);
    }
}
