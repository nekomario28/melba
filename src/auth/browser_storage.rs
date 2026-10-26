//! Read current Chromium records from a bounded in-memory copy. Never lock,
//! recover, compact or otherwise modify the user's live browser database.
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusty_leveldb::{in_memory, LdbIterator, Options, DB};

const MUSIC_ORIGIN: &[u8] = b"_https://music.apple.com\0";
const MAX_SNAPSHOT: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 1024;

fn database_file(name: &str) -> bool {
    name == "CURRENT"
        || name.strip_prefix("MANIFEST-").is_some_and(digits)
        || name
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| digits(stem) && matches!(ext, "log" | "ldb" | "sst"))
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn read_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

fn snapshot(path: &Path) -> Option<DB> {
    let options = Options {
        create_if_missing: false,
        paranoid_checks: true,
        block_cache_capacity_bytes: 1024 * 1024,
        ..in_memory()
    };
    let env = &options.env;
    let mut remaining = MAX_SNAPSHOT;
    let mut count = 0;
    let current = read_file(&path.join("CURRENT"), 1024)?;
    for entry in fs::read_dir(path).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_str()?;
        if !database_file(name) {
            continue;
        }
        count += 1;
        if count > MAX_FILES || !entry.file_type().ok()?.is_file() {
            return None;
        }
        let bytes = read_file(&entry.path(), remaining)?;
        remaining = remaining.checked_sub(bytes.len() as u64)?;
        env.open_writable_file(&Path::new("browser").join(name))
            .ok()?
            .write_all(&bytes)
            .ok()?;
    }
    // A concurrent manifest switch is not a snapshot to import from. Retry on
    // the next poll instead of falling back to raw bytes or obsolete tables.
    if current != read_file(&path.join("CURRENT"), 1024)? {
        return None;
    }
    DB::open("browser", options).ok()
}

/// Chromium's DOMString tag: 1 = Latin-1 (tokens/keys are ASCII), 0 = UTF-16LE.
fn dom_string(bytes: &[u8]) -> Option<String> {
    let (&encoding, bytes) = bytes.split_first()?;
    match encoding {
        1 => Some(bytes.iter().copied().map(char::from).collect()),
        0 if bytes.len().is_multiple_of(2) => String::from_utf16(
            &bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
        .ok(),
        _ => None,
    }
}

fn user_token_key(key: &str) -> bool {
    key.contains("user-token") || key.contains("userToken")
}

pub(super) fn tokens(profile: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(mut db) = snapshot(&profile.join("Local Storage/leveldb")) {
        if let Ok(mut iter) = db.new_iter() {
            iter.seek(MUSIC_ORIGIN);
            while let Some((key, value)) = iter.current() {
                let Some(key) = key.strip_prefix(MUSIC_ORIGIN) else {
                    break;
                };
                if dom_string(key).is_some_and(|k| user_token_key(&k)) {
                    if let Some(value) = dom_string(&value) {
                        if let Some(token) = super::find_jwt(value.as_bytes()) {
                            found.push(token);
                        }
                    }
                }
                if !iter.advance() {
                    break;
                }
            }
        }
    }
    found
}

#[cfg(test)]
pub(super) fn fixture(path: &Path, edit: impl FnOnce(&mut DB)) {
    let options = in_memory();
    let env = options.env.clone();
    let mut db = DB::open("fixture", options).unwrap();
    edit(&mut db);
    db.close().unwrap();
    fs::create_dir_all(path).unwrap();
    for name in env.children(Path::new("fixture")).unwrap() {
        if !database_file(name.to_str().unwrap()) {
            continue;
        }
        let mut bytes = Vec::new();
        env.open_sequential_file(&Path::new("fixture").join(&name))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        fs::write(path.join(name), bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_strings_follow_chromium_encoding_tags() {
        assert_eq!(dom_string(b"\x01caf\xe9").as_deref(), Some("café"));
        assert_eq!(
            dom_string(b"\x00c\x00a\x00f\x00\xe9\x00").as_deref(),
            Some("café")
        );
        assert!(dom_string(b"\x00x").is_none());
        assert!(dom_string(b"\x02ignored").is_none());
    }

    #[test]
    fn reads_real_chromium_records_but_does_not_trust_their_origin() {
        // Captured from a new headless Chromium-family profile on loopback.
        // The fake token is deliberately unsigned; no real account was used.
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/auth/chromium-localstorage");
        let mut db = snapshot(&path)
            .expect("must read the real browser format, not reject an empty fixture");
        let key = b"_http://127.0.0.1:32821\0\x01media-user-token";
        let value = db
            .get(key)
            .expect("the browser must have written the fake token");
        let decoded = dom_string(&value).unwrap();
        assert_eq!(
            decoded,
            "eyJhbGciOiJFUzI1NiJ9.eyJleHAiOjk5OTk5OTk5OTksImlzcyI6ImF1ZGl0LWZpeHR1cmUifQ.QUJDQUJD"
        );
        let mut iter = db.new_iter().unwrap();
        iter.seek(MUSIC_ORIGIN);
        assert!(iter.current().is_none());
    }
}
