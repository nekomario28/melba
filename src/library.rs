//! Slice 02: library + importers producing `sone.collection/v1`.
//! Local files become queryable tracks; XSPF / iTunes XML / M3U playlists
//! normalize to the same contract. No Apple account anywhere.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub props: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub schema: String, // "sone.collection/v1"
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    pub imported_at: String,
    pub provenance: Provenance,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub kind: String,
    pub locator: String,
    /// Some producers (musickit::collection_of) omit the timestamp.
    #[serde(default)]
    pub retrieved_at: String,
    pub tool: String,
}

/// ISO-8601 UTC timestamp without chrono — shared by every
/// sone.collection/v1 producer so `importedAt` formats stay identical.
pub(crate) fn now_iso() -> String {
    // ISO-8601 UTC without chrono: seconds since epoch → civil.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let rem = secs % 86400;
    let (y, m, d);
    // days -> y/m/d (Howard Hinnant civil_from_days)
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let yy = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    d = doy - (153 * mp + 2) / 5 + 1;
    m = if mp < 10 { mp + 3 } else { mp - 9 };
    y = if m <= 2 { yy + 1 } else { yy };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn stable_id(seed: &str) -> String {
    let h = Sha256::digest(seed.as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("local:{}", &hex[..13])
}

pub fn collection(name: &str, tracks: Vec<Track>, kind: &str, locator: &str) -> Collection {
    Collection {
        schema: "sone.collection/v1".into(),
        id: stable_id(locator),
        name: name.to_string(),
        tracks,
        imported_at: now_iso(),
        provenance: Provenance {
            kind: kind.into(),
            locator: locator.into(),
            retrieved_at: now_iso(),
            tool: format!("melbad/{}", env!("CARGO_PKG_VERSION")),
        },
    }
}

// ---- directory scan --------------------------------------------------------

const AUDIO_EXT: &[&str] = &[
    "flac", "alac", "m4a", "wav", "aif", "aiff", "mp3", "ogg", "opus", "aac",
];

pub fn scan_dir(dir: &Path) -> Result<Collection, String> {
    let mut tracks = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    // canonicalized dirs dedupe symlink cycles — a `p.is_dir()` check
    // follows links, so without a visited set dir->link->parent loops forever
    let mut seen = std::collections::HashSet::new();
    while let Some(d) = stack.pop() {
        let real = std::fs::canonicalize(&d).unwrap_or_else(|_| d.clone());
        if !seen.insert(real) {
            continue;
        }
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| AUDIO_EXT.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
            {
                if let Some(t) = track_from_file(&p) {
                    tracks.push(t);
                }
            }
        }
    }
    tracks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(collection(
        &format!("{} (scan)", dir.display()),
        tracks,
        "directory-scan",
        &format!("file://{}", dir.display()),
    ))
}

fn track_from_file(p: &Path) -> Option<Track> {
    let uri = format!("file://{}", p.canonicalize().ok()?.display());
    let src = crate::audio::probe_source(p.to_str()?).ok();
    let mut props = std::collections::BTreeMap::new();
    let mut title = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string();
    let mut artist = None;
    let mut album = None;
    let mut dur = None;
    if let Some(s) = &src {
        props.insert("rate".into(), s.rate.to_string());
        props.insert("bits".into(), s.bits.to_string());
        props.insert("codec".into(), s.codec.clone());
        if let Some((t, a, al, d)) = read_tags(p) {
            if let Some(x) = t {
                title = x;
            }
            artist = a;
            album = al;
            dur = d;
        }
    }
    Some(Track {
        id: stable_id(&uri),
        title,
        artist,
        album,
        duration_ms: dur,
        play_uri: Some(uri.clone()),
        source: Some("local-file".into()),
        props: if props.is_empty() { None } else { Some(props) },
    })
}

/// Embedded metadata + cover art read from a local audio file.
#[derive(Default)]
pub struct FileMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub dur_ms: Option<u64>,
    /// (file extension, raw image bytes) of the first embedded visual.
    pub cover: Option<(String, Vec<u8>)>,
}

/// Read embedded tags and the first embedded cover visual via symphonia.
pub fn file_meta(p: &Path) -> Option<FileMeta> {
    let file = std::fs::File::open(p).ok()?;
    let mss = symphonia::core::io::MediaSourceStream::new(Box::new(file), Default::default());
    let mut probed = symphonia::default::get_probe()
        .format(
            &Default::default(),
            mss,
            &symphonia::core::formats::FormatOptions::default(),
            &symphonia::core::meta::MetadataOptions::default(),
        )
        .ok()?;
    let mut meta = FileMeta {
        dur_ms: probed
            .format
            .default_track()
            .and_then(|t| t.codec_params.n_frames)
            .and_then(|nf| {
                probed
                    .format
                    .default_track()
                    .and_then(|t| t.codec_params.sample_rate)
                    // u128 so a pathological n_frames can't wrap the product
                    .map(|sr| ((nf as u128) * 1000 / sr as u128).min(u64::MAX as u128) as u64)
            }),
        ..Default::default()
    };
    // NB: `probed.metadata` (the fallback log) is empty for MP4/FLAC —
    // embedded tags live on the format reader's metadata revisions.
    let m = probed.format.metadata();
    {
        if let Some(rev) = m.current() {
            use symphonia::core::meta::StandardTagKey as K;
            for tag in rev.tags() {
                if let Some(k) = tag.std_key {
                    let v = tag.value.to_string();
                    match k {
                        K::TrackTitle => meta.title = Some(v),
                        K::Artist => meta.artist = Some(v),
                        K::Album => meta.album = Some(v),
                        _ => {}
                    }
                }
            }
            for vis in rev.visuals() {
                let ext = match vis.media_type.as_str() {
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/png" => "png",
                    _ => continue,
                };
                if !vis.data.is_empty() {
                    meta.cover = Some((ext.to_string(), vis.data.to_vec()));
                    break;
                }
            }
        }
    }
    Some(meta)
}

type RawTags = (Option<String>, Option<String>, Option<String>, Option<u64>);

fn read_tags(p: &Path) -> Option<RawTags> {
    file_meta(p).map(|m| (m.title, m.artist, m.album, m.dur_ms))
}

// ---- playlist importers ----------------------------------------------------

pub fn import(path: &Path) -> Result<Collection, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "xspf" => import_xspf(path),
        "xml" | "itunes" => import_itunes(path),
        "m3u" | "m3u8" => import_m3u(path),
        "json" => serde_json::from_str::<Collection>(
            &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("bad sone.collection json: {e}")),
        _ if path.is_dir() => scan_dir(path),
        _ => Err(format!("unknown playlist format: {}", path.display())),
    }
}

fn import_xspf(path: &Path) -> Result<Collection, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let doc = roxmltree::Document::parse(&text).map_err(|e| format!("xml: {e}"))?;
    let mut tracks = Vec::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("track")) {
        let get = |name| {
            n.children()
                .find(|c| c.has_tag_name(name))
                .and_then(|c| c.text())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        let loc = get("location").unwrap_or_default();
        tracks.push(Track {
            id: stable_id(&loc),
            title: get("title").unwrap_or_else(|| loc.clone()),
            artist: get("creator"),
            album: get("album"),
            duration_ms: get("duration").and_then(|d| d.parse().ok()),
            play_uri: if loc.is_empty() { None } else { Some(loc) },
            source: Some("xspf".into()),
            props: None,
        });
    }
    Ok(collection(
        &path.file_stem().unwrap_or_default().to_string_lossy(),
        tracks,
        "xspf",
        &format!("file://{}", path.display()),
    ))
}

fn import_m3u(path: &Path) -> Result<Collection, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut tracks = Vec::new();
    let mut pending_title: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("#EXTINF:") {
            pending_title = rest
                .split_once(',')
                .map(|x| x.1)
                .map(|s| s.trim().to_string());
            continue;
        }
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        let loc = if l.starts_with("http://") || l.starts_with("https://") || l.starts_with("file:")
        {
            l.to_string()
        } else {
            format!("file://{}", base.join(l).display())
        };
        tracks.push(Track {
            id: stable_id(&loc),
            title: pending_title.take().unwrap_or_else(|| {
                Path::new(&loc)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| loc.clone())
            }),
            artist: None,
            album: None,
            duration_ms: None,
            play_uri: Some(loc),
            source: Some("m3u".into()),
            props: None,
        });
    }
    Ok(collection(
        &path.file_stem().unwrap_or_default().to_string_lossy(),
        tracks,
        "m3u",
        &format!("file://{}", path.display()),
    ))
}

fn import_itunes(path: &Path) -> Result<Collection, String> {
    let v = plist::Value::from_file(path).map_err(|e| format!("plist: {e}"))?;
    let root = v.as_dictionary().ok_or("not a plist dict")?;
    let tracks_obj = root
        .get("Tracks")
        .and_then(|t| t.as_dictionary())
        .ok_or("no Tracks dict")?;
    let mut tracks = Vec::new();
    for (_k, tv) in tracks_obj {
        let d = match tv.as_dictionary() {
            Some(d) => d,
            None => continue,
        };
        let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(String::from);
        let loc = s("Location").unwrap_or_default();
        let tid = s("Persistent ID").unwrap_or_else(|| loc.clone());
        tracks.push(Track {
            id: format!("itunes:{}", tid),
            title: s("Name").unwrap_or_else(|| loc.clone()),
            artist: s("Artist"),
            album: s("Album"),
            duration_ms: d.get("Total Time").and_then(|v| v.as_unsigned_integer()),
            play_uri: if loc.is_empty() { None } else { Some(loc) },
            source: Some("itunes-xml".into()),
            props: None,
        });
    }
    Ok(collection(
        &path.file_stem().unwrap_or_default().to_string_lossy(),
        tracks,
        "itunes-xml",
        &format!("file://{}", path.display()),
    ))
}

/// Serialize to the sone.collection/v1 wire shape (camelCase, contract field
/// names) and validate against the schema when present.
pub fn to_collection_json(c: &Collection) -> Result<String, String> {
    // Collection serializes to the contract shape directly.
    serde_json::to_string_pretty(c).map_err(|e| e.to_string())
}

pub fn validate_against_schema(path: &Path, json: &str) -> Result<(), String> {
    let schema_text = std::fs::read_to_string(path).map_err(|e| format!("schema read: {e}"))?;
    let schema: serde_json::Value =
        serde_json::from_str(&schema_text).map_err(|e| format!("schema parse: {e}"))?;
    let doc: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let v = jsonschema::draft202012::new(&schema).map_err(|e| e.to_string())?;
    let mut errs = Vec::new();
    for e in v.iter_errors(&doc) {
        errs.push(e.to_string());
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(p: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(p)
    }

    fn check_schema(col: &Collection) {
        let j = to_collection_json(col).expect("serialize");
        validate_against_schema(&repo("contracts/sone.collection.v1.schema.json"), &j)
            .expect("schema valid");
    }

    #[test]
    fn xspf_import_validates() {
        let c = import(&repo("fixtures/playlist.xspf")).expect("import xspf");
        assert!(!c.tracks.is_empty());
        check_schema(&c);
    }

    #[test]
    fn m3u_import_validates() {
        let c = import(&repo("fixtures/list.m3u")).expect("import m3u");
        assert!(!c.tracks.is_empty());
        check_schema(&c);
    }

    #[test]
    fn itunes_import_validates() {
        let c = import(&repo("fixtures/itunes.xml")).expect("import itunes");
        assert!(!c.tracks.is_empty());
        check_schema(&c);
    }

    #[test]
    fn dir_scan_validates_with_rate_props() {
        let c = scan_dir(&repo("fixtures/lib")).expect("scan");
        assert!(!c.tracks.is_empty());
        check_schema(&c);
        let any_rate = c.tracks.iter().any(|t| {
            t.props
                .as_ref()
                .map(|p| p.contains_key("rate"))
                .unwrap_or(false)
        });
        assert!(any_rate, "rate/bit props expected on at least one track");
    }

    #[test]
    fn collection_schema_id_is_frozen() {
        let c = collection("t", vec![], "test", "mem");
        assert_eq!(c.schema, "sone.collection/v1");
    }
}
