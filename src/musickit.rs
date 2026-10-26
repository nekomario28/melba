//! Slice 05: MusicKit metadata enrichment — catalog-only, no playback.
//!
//! The developer token is fetched from Apple's public storefront page
//! (music.apple.com embeds it for MusicKit JS); the same thing Cider and
//! every browser-based client does. No user Apple ID is touched.
//!
//! /v1/catalog/* is public catalog metadata (search, album
//! lookup, artwork URL, audioVariants flags like lossless/hi-res-lossless).
//! /v1/me/* (library, personal endpoints) needs a music-user-token issued by
//! Apple's own sign-in flow (`melbad auth login`); account access is separate.

use serde_json::{json, Value};

/// Catalog storefront for app-originated requests (search, preview,
/// `melbad mk`). `MELBA_STORE` overrides; default `jp` for this client. Deep-links
/// carry their own storefront and use the `*_in` variants — catalog IDs are
/// storefront-scoped, so a `/us/` link resolved against `/jp/` 404s.
pub fn storefront() -> &'static str {
    static S: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    S.get_or_init(|| std::env::var("MELBA_STORE").unwrap_or_else(|_| "jp".into()))
}
const TOKEN_PAGE: &str = "https://music.apple.com/us";
const API: &str = "https://api.music.apple.com/v1/catalog";
/// the web player's own catalog host — serves `extendedAssetUrls` and
/// `lyrics` where api.music.apple.com silently drops or 40012-denies them
const AMP_API: &str = "https://amp-api-edge.music.apple.com/v1/catalog";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!(
            "melba/",
            env!("CARGO_PKG_VERSION"),
            " (musickit catalog metadata)"
        ))
        // HTTP_PROXY / HTTPS_PROXY / ALL_PROXY are the one place a proxy is
        // configured: the app never asks for it in its own interface, and the
        // daemon must honour the same environment the GUI does.
        .try_proxy_from_env(true)
        .build()
}

/// Fetch Apple's public MusicKit developer token. The storefront HTML links
/// an app JS bundle (/assets/index~*.js) that embeds several JWTs; the
/// catalog-valid one has iss == "AMPWebPlay" (verified 200 on api.music).
/// Cached under ~/.cache/melba/dev-token until its JWT exp approaches.
pub fn dev_token() -> Result<String, String> {
    if let Some(t) = cached_token() {
        return Ok(t);
    }
    let t = dev_token_fetch()?;
    let _ = cache_token(&t);
    Ok(t)
}

fn token_cache_path() -> Option<std::path::PathBuf> {
    crate::config::xdg_dir("XDG_CACHE_HOME", ".cache").map(|p| p.join("melba").join("dev-token"))
}

fn cached_token() -> Option<String> {
    let t = std::fs::read_to_string(token_cache_path()?)
        .ok()?
        .trim()
        .to_string();
    let exp = crate::auth::exp_of(&t)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if exp > now + 300 {
        Some(t)
    } else {
        None
    }
}

fn cache_token(t: &str) -> Result<(), String> {
    let p = token_cache_path().ok_or("no XDG_CACHE_HOME or HOME")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(p, t).map_err(|e| e.to_string())
}

/// Drop the cached developer token. Apple can revoke a JWT before its
/// `exp` claim; callers that hit a 401 should invalidate and retry so a
/// dead-but-unexpired cache entry does not brick every request.
pub fn invalidate_dev_token() {
    if let Some(p) = token_cache_path() {
        let _ = std::fs::remove_file(p);
    }
}

fn dev_token_fetch() -> Result<String, String> {
    let a = agent();
    let html = a
        .get(TOKEN_PAGE)
        .call()
        .map_err(|e| format!("token page: {e}"))?
        .into_string()
        .map_err(|e| format!("token body: {e}"))?;
    // locate the app bundle
    let js_path = extract_js_path(&html).ok_or("no /assets/index~*.js on storefront")?;
    let js = a
        .get(&format!("https://music.apple.com{js_path}"))
        .call()
        .map_err(|e| format!("js bundle: {e}"))?
        .into_string()
        .map_err(|e| format!("js body: {e}"))?;
    let mut first_jwt = None;
    for m in js.match_indices("eyJ") {
        let cand: String = js[m.0..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
            .collect();
        if cand.matches('.').count() != 2 || cand.len() < 100 {
            continue;
        }
        if let Some(payload) = cand.split('.').nth(1) {
            if let Ok(bytes) = b64url_decode(payload) {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    match v["iss"].as_str() {
                        Some("AMPWebPlay") => return Ok(cand),
                        Some(_) if first_jwt.is_none() => first_jwt = Some(cand),
                        _ => {}
                    }
                }
            }
        }
    }
    first_jwt.ok_or("no developer token found in app bundle".into())
}

fn extract_js_path(html: &str) -> Option<String> {
    for needle in ["/assets/index~", "/assets/index."] {
        if let Some(i) = html.find(needle) {
            let rest = &html[i..];
            let end = rest.find('"').unwrap_or(rest.len());
            let p = &rest[..end];
            if p.ends_with(".js") {
                return Some(p.to_string());
            }
        }
    }
    None
}

/// Probe a media-user-token against the catalog — a stale cookie stays
/// in the jar after Apple kills the session server-side, and this is the
/// only way to tell live from dead. Returns Ok on HTTP 200.
pub fn validate_token(mtok: &str) -> Result<(), String> {
    let dev = dev_token()?;
    agent()
        .get("https://api.music.apple.com/v1/me/storefront")
        .set("Authorization", &format!("Bearer {dev}"))
        .set("Origin", "https://music.apple.com")
        .set("media-user-token", mtok)
        .call()
        .map(|_| ())
        .map_err(|e| format!("token rejected: {e}"))
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    // minimal base64url decoder (no padding)
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

fn get(path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    get_in(storefront(), path, query)
}

/// Run `f` with the cached dev token; on a 401 the token may have been
/// revoked before `exp` — invalidate, refetch, retry once. Shared by
/// every catalog call (search, lookup, previews, lyrics, /me) so a
/// stale token can't poison one path while the others recover.
fn with_dev_retry<T>(f: impl Fn(&str) -> Result<T, String>) -> Result<T, String> {
    let t = dev_token()?;
    match f(&t) {
        Err(e) if e.contains("401") => f(&{
            invalidate_dev_token();
            dev_token()?
        }),
        r => r,
    }
}

fn get_in(store: &str, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    with_dev_retry(|token| {
        let url = format!("{API}/{store}{path}");
        let mut req = agent()
            .get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Origin", "https://music.apple.com");
        for (k, v) in query {
            req = req.query(k, v);
        }
        let res = req.call().map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&res.into_string().map_err(|e| e.to_string())?)
            .map_err(|e| format!("json: {e}"))
    })
}

/// Summarize a catalog resource into {id, name, artistName?, artwork, audioVariants?}.
fn summarize(item: &Value) -> Value {
    let a = &item["attributes"];
    let mut out = json!({
        "id": item["id"],
        "type": item["type"],
        "name": a["name"],
    });
    if let Some(v) = a.get("artistName") {
        out["artistName"] = v.clone();
    }
    if let Some(v) = a.get("albumName") {
        out["albumName"] = v.clone();
    }
    // playlists carry a curator instead of an artist
    if let Some(v) = a.get("curatorName") {
        out["curatorName"] = v.clone();
    }
    if let Some(v) = a.get("audioVariants") {
        out["audioVariants"] = v.clone();
    }
    if let Some(v) = a.get("hasLyrics") {
        out["hasLyrics"] = v.clone();
    }
    if let Some(v) = a.get("durationInMillis") {
        out["durationMs"] = v.clone();
    }
    if let Some(v) = a.get("releaseDate") {
        out["releaseDate"] = v.clone();
    }
    if let Some(v) = a.get("trackCount") {
        out["trackCount"] = v.clone();
    }
    if let Some(v) = a.get("genreNames") {
        out["genreNames"] = v.clone();
    }
    if let Some(w) = a.get("artwork") {
        // Apple artwork url template: {w}x{h}bb.jpg
        if let Some(u) = w.get("url").and_then(|s| s.as_str()) {
            out["artworkUrl"] = json!(u.replace("{w}", "512").replace("{h}", "512"));
        }
    }
    out
}

pub fn search(term: &str, types: &str, limit: usize) -> Result<Value, String> {
    let v = get(
        "/search",
        &[
            ("term", term),
            ("types", types),
            ("limit", &limit.to_string()),
            ("extend", "audioVariants"),
        ],
    )?;
    let mut out = json!({"results": {}});
    for (kind, group) in v["results"].as_object().cloned().unwrap_or_default() {
        let data = group["data"].as_array().cloned().unwrap_or_default();
        out["results"][kind] = json!(data.iter().map(summarize).collect::<Vec<_>>());
    }
    Ok(out)
}

pub fn lookup_song(id: &str) -> Result<Value, String> {
    lookup_song_in(storefront(), id)
}

pub fn lookup_song_in(store: &str, id: &str) -> Result<Value, String> {
    let v = get_in(
        store,
        &format!("/songs/{id}"),
        &[("extend", "audioVariants"), ("include", "albums")],
    )?;
    let d = &v["data"][0];
    if d.is_null() {
        return Err(format!("song {id} not found"));
    }
    let mut out = summarize(d);
    if let Some(a) = d["relationships"]["albums"]["data"][0]["id"].as_str() {
        out["albumId"] = json!(a);
    }
    Ok(out)
}

/// Header of any catalog resource — `kind` is the path segment
/// (songs | albums | artists | playlists). Flat `summarize` shape.
pub fn lookup(kind: &str, id: &str) -> Result<Value, String> {
    lookup_in(storefront(), kind, id)
}

pub fn lookup_in(store: &str, kind: &str, id: &str) -> Result<Value, String> {
    let v = get_in(store, &format!("/{kind}/{id}"), &[])?;
    let d = &v["data"][0];
    if d.is_null() {
        return Err(format!("{kind}/{id} not found"));
    }
    Ok(summarize(d))
}

pub fn lookup_album(id: &str) -> Result<Value, String> {
    lookup("albums", id)
}

/// GET /v1/catalog/{store}/charts — public storefront charts, dev token only.
/// Returns `{songs: [summarized], albums: [summarized]}`.
pub fn charts(store: &str, limit: usize) -> Result<Value, String> {
    let l = limit.to_string();
    let v = get_in(
        store,
        "/charts",
        &[("types", "songs,albums"), ("limit", &l)],
    )?;
    let mut out = json!({});
    for kind in ["songs", "albums"] {
        if let Some(data) = v["results"][kind][0]["data"].as_array() {
            out[kind] = json!(data.iter().map(summarize).collect::<Vec<_>>());
        }
    }
    Ok(out)
}

/// Artist header plus the two views music.apple.com shows on an artist page:
/// `full-albums` and `top-songs` (both summarized like search results).
pub fn artist(id: &str) -> Result<Value, String> {
    artist_in(storefront(), id)
}

pub fn artist_in(store: &str, id: &str) -> Result<Value, String> {
    let v = get_in(
        store,
        &format!("/artists/{id}"),
        &[
            ("views", "top-songs,full-albums"),
            ("extend", "audioVariants"),
        ],
    )?;
    let d = &v["data"][0];
    if d.is_null() {
        return Err(format!("artist {id} not found"));
    }
    let mut out = summarize(d);
    for (view, key) in [("full-albums", "albums"), ("top-songs", "topSongs")] {
        out[key] = json!(d["views"][view]["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(summarize)
            .collect::<Vec<_>>());
    }
    Ok(out)
}

/// Enrich a sone.collection/v1 doc in place-ish: for each track with an
/// artist name, search the catalog and attach best-match props.
pub fn enrich_collection(json_text: &str, limit: usize) -> Result<String, String> {
    let mut doc: Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;
    // fail fast on a missing token rather than "enriching" an unchanged doc
    dev_token()?;
    let tracks = doc["tracks"].as_array_mut().ok_or("no tracks array")?;
    let mut done = std::collections::HashSet::new();
    for t in tracks.iter_mut().take(limit) {
        let title = t["title"].as_str().unwrap_or("").to_string();
        let artist = t["artist"].as_str().unwrap_or("").to_string();
        if title.is_empty() || !done.insert(format!("{artist}\x00{title}")) {
            continue;
        }
        let q = format!("{artist} {title}");
        let url = format!("{API}/{}/search", storefront());
        let res = with_dev_retry(|token| {
            agent()
                .get(&url)
                .set("Authorization", &format!("Bearer {token}"))
                .set("Origin", "https://music.apple.com")
                .query("term", &q)
                .query("types", "songs")
                .query("limit", "1")
                .query("extend", "audioVariants")
                .call()
                .map_err(|e| e.to_string())
        });
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                eprintln!("enrich: {q}: {e}");
                continue;
            }
        };
        let v: Value = match serde_json::from_str(&res.into_string().unwrap_or_default()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("enrich: {q}: json {e}");
                continue;
            }
        };
        let song = v["results"]["songs"]["data"][0].clone();
        if song.is_null() {
            eprintln!("enrich: {q}: no catalog match");
            continue;
        }
        let s = summarize(&song);
        if !t["props"].is_object() {
            // a non-object props value (string/array in the input doc)
            // is replaced wholesale rather than merged or panicked on
            t["props"] = json!({});
        }
        let Some(entry) = t["props"].as_object_mut() else {
            continue;
        };
        entry.insert(
            "apple.catalogId".into(),
            json!(s["id"].as_str().unwrap_or("")),
        );
        if let Some(vars) = s.get("audioVariants").and_then(|v| v.as_array()) {
            entry.insert(
                "apple.audioVariants".into(),
                json!(vars
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(",")),
            );
        }
        if let Some(u) = s.get("artworkUrl").and_then(|v| v.as_str()) {
            entry.insert("apple.artwork".into(), json!(u));
        }
    }
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

// ---- user-library endpoints (need music-user-token) ------------------------

fn me_get(path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let ut = crate::auth::get()
        .ok_or("no music-user-token — run `melbad auth open` (see `melbad auth --help`)")?;
    with_dev_retry(|dt| {
        let url = format!("https://api.music.apple.com/v1/me{path}");
        let mut req = agent()
            .get(&url)
            .set("Authorization", &format!("Bearer {dt}"))
            .set("Music-User-Token", &ut)
            .set("Origin", "https://music.apple.com");
        for (k, v) in query {
            req = req.query(k, v);
        }
        let res = req.call().map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&res.into_string().map_err(|e| e.to_string())?)
            .map_err(|e| format!("json: {e}"))
    })
}

fn me_post(path: &str, query: &[(&str, String)]) -> Result<(), String> {
    let ut = crate::auth::get().ok_or("no music-user-token — `melbad auth set <token>`")?;
    with_dev_retry(|dt| {
        let url = format!("https://api.music.apple.com/v1/me{path}");
        let mut req = agent()
            .post(&url)
            .set("Authorization", &format!("Bearer {dt}"))
            .set("Music-User-Token", &ut)
            .set("Origin", "https://music.apple.com");
        for (k, v) in query {
            req = req.query(k, v);
        }
        req.call().map_err(|e| format!("{path}: {e}"))?;
        Ok(())
    })
}

/// GET /v1/me/library/<type>[?ids=...] or /v1/me/recent/played/tracks etc.
/// `what`: library-albums | library-songs | library-playlists |
///         recent-tracks | recommendations | heavy-rotation
pub fn me(what: &str, limit: usize) -> Result<Value, String> {
    let l = limit.to_string();
    let (path, q): (String, Vec<(&str, &str)>) = match what {
        "library-albums" => ("/library/albums".into(), vec![("limit", l.as_str())]),
        "library-songs" => ("/library/songs".into(), vec![("limit", l.as_str())]),
        "library-playlists" => ("/library/playlists".into(), vec![("limit", l.as_str())]),
        "recent-tracks" => ("/recent/played/tracks".into(), vec![("limit", l.as_str())]),
        "recent-albums" => (
            "/recent/played".into(),
            vec![("types", "albums"), ("limit", l.as_str())],
        ),
        "recommendations" => ("/recommendations".into(), vec![("limit", l.as_str())]),
        "heavy-rotation" => (
            "/history/heavy-rotation".into(),
            vec![("limit", l.as_str())],
        ),
        _ => return Err(format!("unknown `mk me {what}`")),
    };
    let v = me_get(&path, &q)?;
    let mut out = json!({"items": []});
    if let Some(arr) = v["data"].as_array() {
        out["items"] = json!(arr.iter().map(summarize).collect::<Vec<_>>());
    }
    Ok(out)
}

/// Add a catalog item to the user's library — official write endpoint.
/// `rtype`: songs | albums | playlists
pub fn library_add(rtype: &str, id: &str) -> Result<(), String> {
    me_post("/library", &[(&format!("ids[{rtype}]"), id.to_string())])
}

/// Song lyrics. The catalog lyrics endpoint requires a permission the public
/// AMPWebPlay token does not carry (verified: 40012 Insufficient Permissions);
/// it succeeds with a music-user-token present — honest error otherwise.
pub fn lyrics(song_id: &str) -> Result<Value, String> {
    lyrics_in(storefront(), song_id)
}

pub fn lyrics_in(store: &str, song_id: &str) -> Result<Value, String> {
    with_dev_retry(|dt| {
        let url = format!("{AMP_API}/{store}/songs/{song_id}/lyrics");
        let mut req = agent()
            .get(&url)
            .set("Authorization", &format!("Bearer {dt}"))
            .set("Origin", "https://music.apple.com");
        if let Some(ut) = crate::auth::get() {
            req = req.set("Music-User-Token", &ut);
        }
        let res = req.call().map_err(|e| format!("lyrics: {e}"))?;
        let v: Value = serde_json::from_str(&res.into_string().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut out = json!({"id": song_id});
        if let Some(d) = v["data"].as_array().and_then(|a| a.first()) {
            out["lyrics"] = d["attributes"]["ttml"].clone();
            out["playParams"] = d["attributes"]["playParams"].clone();
            Ok(out)
        } else {
            Err(format!("lyrics: {}", v["errors"][0]["detail"]))
        }
    })
}

/// TTML lyrics → displayable plain text: paragraph/line boundaries become
/// newlines, every other tag drops, entities decode to utf-8.
pub fn ttml_to_text(ttml: &str) -> String {
    let mut out = String::with_capacity(ttml.len());
    let b = ttml.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            let end = ttml[i..].find('>').map(|e| i + e);
            if let Some(e) = end {
                let tag = &ttml[i + 1..e];
                let name = tag
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .next()
                    .unwrap_or("");
                if matches!(name, "p" | "br" | "div" | "tt")
                    && !out.ends_with('\n')
                    && !out.is_empty()
                {
                    out.push('\n');
                }
                i = e + 1;
                continue;
            }
        }
        if b[i] == b'&' {
            if let Some(semi) = ttml[i..].find(';') {
                let ent = &ttml[i + 1..i + semi];
                let dec = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                        .ok()
                        .and_then(char::from_u32),
                    _ if ent.starts_with('#') => {
                        ent[1..].parse::<u32>().ok().and_then(char::from_u32)
                    }
                    _ => None,
                };
                if let Some(c) = dec {
                    out.push(c);
                    i += semi + 1;
                    continue;
                }
            }
        }
        // UTF-8 safe: push the full char, not the byte
        let c = ttml[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    // per-line trim + drop whitespace-only runs (tag indentation) — blank
    // lines from empty <p/> elements collapse away, which is fine for v1
    out.trim()
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One timed lyrics line — Apple's `lyrics_line`/`lyrics_line_karaoke`.
/// `words` stays empty for `itunes:timing="Line"` tracks; "Word" tracks
/// fill it from inner `<span begin= end=>…</span>` (lyrics_word_karaoke).
#[derive(Clone, Debug)]
pub struct LyricLine {
    pub begin_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub words: Vec<(i64, i64, String)>,
}

/// "17.509" | "1:06.776" | "1:23:45.5" → milliseconds
fn ttml_ms(s: &str) -> Option<i64> {
    let mut parts = s.trim().split(':');
    let secs: f64 = parts.next_back()?.parse().ok()?;
    let mut total = secs;
    if let Some(m) = parts.next_back() {
        total += m.parse::<f64>().ok()? * 60.0;
    }
    if let Some(h) = parts.next_back() {
        total += h.parse::<f64>().ok()? * 3600.0;
    }
    Some((total * 1000.0) as i64)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let e = tag[i..].find('"')? + i;
    Some(&tag[i..e])
}

/// TTML → timed lines. Walks `<p>` elements with begin/end (Apple ships
/// per-line timing even when `timing="Line"`); inner `<span>`s keep their
/// own timing for word-level karaoke when the track ships it.
pub fn ttml_to_lines(ttml: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let b = ttml.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(e) = ttml[i..].find('>').map(|e| i + e) else {
            break;
        };
        let tag = &ttml[i + 1..e];
        i = e + 1;
        // only opening <p …> with a begin attr starts a line
        if !tag.starts_with("p ") && !tag.starts_with("p\t") {
            continue;
        }
        let (Some(begin), Some(end)) = (attr(tag, "begin"), attr(tag, "end")) else {
            continue;
        };
        let (Some(bms), Some(ems)) = (ttml_ms(begin), ttml_ms(end)) else {
            continue;
        };
        // inner content until </p>: text + optional timed word spans
        let close = ttml[i..].find("</p>").map(|c| i + c).unwrap_or(ttml.len());
        let inner = &ttml[i..close];
        let mut text = String::new();
        let mut words = Vec::new();
        let mut j = 0;
        let ib = inner.as_bytes();
        while j < ib.len() {
            if ib[j] == b'<' {
                let Some(se) = inner[j..].find('>').map(|x| j + x) else {
                    break;
                };
                let stag = &inner[j + 1..se];
                j = se + 1;
                if stag.starts_with("span ") {
                    if let (Some(wb), Some(we)) = (attr(stag, "begin"), attr(stag, "end")) {
                        if let (Some(wb), Some(we)) = (ttml_ms(wb), ttml_ms(we)) {
                            // word text = content until next tag/close span
                            let we_close = inner[j..]
                                .find("</span>")
                                .map(|c| j + c)
                                .unwrap_or(inner.len());
                            let wt = inner[j..we_close].to_string();
                            j = we_close;
                            if !wt.trim().is_empty() {
                                // spans hold the whole line text — the
                                // outer text run stays empty for these
                                text.push_str(&wt);
                                words.push((wb, we, wt));
                            }
                            continue;
                        }
                    }
                }
                continue;
            }
            let c = inner[j..].chars().next().unwrap();
            text.push(c);
            j += c.len_utf8();
        }
        let text = ttml_to_text(&text); // entity decode via the same path
        if !text.trim().is_empty() {
            lines.push(LyricLine {
                begin_ms: bms,
                end_ms: ems,
                text,
                words: words
                    .into_iter()
                    .map(|(a, b, w)| (a, b, ttml_to_text(&w)))
                    .collect(),
            });
        }
        i = close;
    }
    lines
}

/// Lyrics as displayable text (token-gated — see `lyrics()`).
pub fn lyrics_text(song_id: &str) -> Result<String, String> {
    lyrics_text_in(storefront(), song_id)
}

pub fn lyrics_text_in(store: &str, song_id: &str) -> Result<String, String> {
    let v = lyrics_in(store, song_id)?;
    Ok(ttml_to_text(v["lyrics"].as_str().unwrap_or_default()))
}

// ---- previews + downloads (public, non-DRM 30s AAC) ------------------------

/// `preview:<id>` or `preview:<cc>:<id>` — the catalog-preview URI the daemon
/// resolves. A leading alpha-2 segment is the storefront (deep-links carry
/// theirs); bare ids use the app's own. Catalog ids never contain ':' so the
/// split is unambiguous.
pub fn preview_parts(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("preview:")?;
    match rest.split_once(':') {
        Some((cc, id))
            if cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic()) && !id.is_empty() =>
        {
            Some((cc.to_ascii_lowercase(), id.to_string()))
        }
        // catalog ids never contain ':' — a first segment that isn't a
        // storefront means malformed input, not a weird id
        Some(_) => None,
        None => (!rest.is_empty()).then(|| (storefront().to_string(), rest.to_string())),
    }
}

/// Fetch a song's preview URL (public AAC .m4a, ~30s).
pub fn preview_url(song_id: &str) -> Result<(String, Value), String> {
    preview_url_in(storefront(), song_id)
}

pub fn preview_url_in(store: &str, song_id: &str) -> Result<(String, Value), String> {
    let url = format!("{API}/{store}/songs/{song_id}");
    let v: Value = with_dev_retry(|dt| {
        let res = agent()
            .get(&url)
            .set("Authorization", &format!("Bearer {dt}"))
            .set("Origin", "https://music.apple.com")
            .call()
            .map_err(|e| format!("song: {e}"))?;
        serde_json::from_str(&res.into_string().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    })?;
    let a = &v["data"][0]["attributes"];
    let url = a["previews"][0]["url"]
        .as_str()
        .ok_or("no preview url")?
        .to_string();
    Ok((url, summarize(&v["data"][0])))
}

fn cache_dir(sub: &str) -> Option<std::path::PathBuf> {
    let d = crate::config::xdg_dir("XDG_CACHE_HOME", ".cache")?
        .join("melba")
        .join(sub);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// Download the catalog preview to ~/.cache/melba/previews/<id>.m4a.
pub fn download_preview(song_id: &str) -> Result<std::path::PathBuf, String> {
    download_preview_in(storefront(), song_id)
}

pub fn download_preview_in(store: &str, song_id: &str) -> Result<std::path::PathBuf, String> {
    let (url, _meta) = preview_url_in(store, song_id)?;
    // The same id can name different songs in different storefronts — the
    // store prefix keeps caches distinct; the default store keeps the legacy
    // filename so existing cache entries still hit.
    let name = if store == storefront() {
        song_id.to_string()
    } else {
        format!("{store}-{song_id}")
    };
    let out = cache_dir("previews")
        .ok_or("no XDG_CACHE_HOME or HOME")?
        .join(format!("{name}.m4a"));
    let mut resp = agent()
        .get(&url)
        .call()
        .map_err(|e| format!("preview dl: {e}"))?
        .into_reader();
    let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp, &mut f).map_err(|e| e.to_string())?;
    Ok(out)
}

/// `melbad dl <id>`: preview + artwork + metadata manifest into
/// ~/.cache/melba/downloads/<id>/. Full protected tracks are FairPlay-DRM and
/// are NOT downloaded — this is the lawful boundary, documented.
pub fn download(song_id: &str) -> Result<Value, String> {
    let (purl, meta) = preview_url(song_id)?;
    let dir = cache_dir("downloads")
        .ok_or("no XDG_CACHE_HOME or HOME")?
        .join(song_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // preview audio
    let pv = dir.join("preview.m4a");
    let mut resp = agent()
        .get(&purl)
        .call()
        .map_err(|e| format!("dl: {e}"))?
        .into_reader();
    std::io::copy(
        &mut resp,
        &mut std::fs::File::create(&pv).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    // artwork
    let mut art_path = None;
    if let Some(u) = meta.get("artworkUrl").and_then(|v| v.as_str()) {
        if let Ok(r) = agent().get(u).call() {
            let ap = dir.join("artwork.jpg");
            let mut rr = r.into_reader();
            if std::io::copy(
                &mut rr,
                &mut std::fs::File::create(&ap).map_err(|e| e.to_string())?,
            )
            .is_ok()
            {
                art_path = Some(ap.to_string_lossy().to_string());
            }
        }
    }

    let manifest = json!({
        "id": song_id,
        "meta": meta,
        "files": {
            "preview": pv.to_string_lossy(),
            "artwork": art_path,
        },
        "note": "catalog preview (non-DRM). Protected full tracks are FairPlay-bound and not downloadable by design.",
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(manifest)
}

/// Emit catalog results as a sone.collection/v1 doc whose tracks carry
/// `preview:<id>` playUris — playable straight through `melbad serve`.
pub fn collection_from(term: &str, limit: usize) -> Result<String, String> {
    let v = search(term, "songs", limit)?;
    let mut tracks = Vec::new();
    for s in v["results"]["songs"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
    {
        let mut t = json!({
            "id": format!("apple:{}", s["id"].as_str().unwrap_or("")),
            "title": s["name"].as_str().unwrap_or(""),
            "artist": s["artistName"].as_str().unwrap_or(""),
            "playUri": format!("preview:{}", s["id"].as_str().unwrap_or("")),
        });
        let mut props = serde_json::Map::new();
        if let Some(vars) = s["audioVariants"].as_array() {
            props.insert(
                "apple.audioVariants".into(),
                json!(vars
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(",")),
            );
        }
        if let Some(u) = s["artworkUrl"].as_str() {
            props.insert("apple.artwork".into(), json!(u));
        }
        props.insert(
            "apple.catalogId".into(),
            json!(s["id"].as_str().unwrap_or("")),
        );
        t["props"] = json!(props);
        tracks.push(t);
    }
    let doc = json!({
        "schema": "sone.collection/v1",
        "id": format!("catalog:search:{}", term.replace(' ', "-")),
        "name": format!("catalog search: {term}"),
        "tracks": tracks,
        "importedAt": crate::library::now_iso(),
        "provenance": {"kind": "musickit-catalog-search", "locator": term, "tool": "melbad"},
    });
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

/// Tracks of a catalog album or playlist as a sone.collection/v1 doc.
/// `kind`: "albums" | "playlists"
pub fn collection_of(kind: &str, id: &str) -> Result<String, String> {
    collection_of_in(storefront(), kind, id)
}

pub fn collection_of_in(store: &str, kind: &str, id: &str) -> Result<String, String> {
    let url = format!("{API}/{store}/{kind}/{id}");
    let v: Value = with_dev_retry(|dt| {
        let res = agent()
            .get(&url)
            .set("Authorization", &format!("Bearer {dt}"))
            .set("Origin", "https://music.apple.com")
            .query("extend", "audioVariants")
            .call()
            .map_err(|e| format!("{kind}/{id}: {e}"))?;
        serde_json::from_str(&res.into_string().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    })?;
    let head = &v["data"][0];
    if head.is_null() {
        return Err(format!("{kind}/{id} not found"));
    }
    let title = head["attributes"]["name"].as_str().unwrap_or("collection");
    let rel = &head["relationships"]["tracks"]["data"];
    let mut tracks = Vec::new();
    if let Some(arr) = rel.as_array() {
        for tr in arr {
            let a = &tr["attributes"];
            let mut props = serde_json::Map::new();
            props.insert(
                "apple.catalogId".into(),
                json!(tr["id"].as_str().unwrap_or("")),
            );
            if let Some(vars) = a["audioVariants"].as_array() {
                props.insert(
                    "apple.audioVariants".into(),
                    json!(vars
                        .iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(",")),
                );
            }
            if let Some(w) = a["artwork"]["url"].as_str() {
                props.insert(
                    "apple.artwork".into(),
                    json!(w.replace("{w}", "512").replace("{h}", "512")),
                );
            }
            tracks.push(json!({
                "id": format!("apple:{}", tr["id"].as_str().unwrap_or("")),
                "title": a["name"].as_str().unwrap_or(""),
                "artist": a["artistName"].as_str().unwrap_or(""),
                "durationMs": a["durationInMillis"].as_i64().unwrap_or(0),
                "playUri": format!("preview:{}", tr["id"].as_str().unwrap_or("")),
                "props": props,
            }));
        }
    }
    let doc = json!({
        "schema": "sone.collection/v1",
        "id": format!("catalog:{kind}:{id}"),
        "name": title,
        "tracks": tracks,
        "importedAt": crate::library::now_iso(),
        "provenance": {"kind": "musickit-catalog", "locator": format!("{kind}/{id}"), "tool": "melbad"},
    });
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_roundtrip() {
        assert_eq!(b64url_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(
            b64url_decode("eyJpc3MiOiJ4In0").unwrap(),
            b"{\"iss\":\"x\"}"
        );
    }

    #[test]
    fn js_path_extraction() {
        let html = r#"<html><script src="/assets/index~abc123.js"></script>"#;
        assert_eq!(extract_js_path(html).unwrap(), "/assets/index~abc123.js");
        assert!(extract_js_path("<html></html>").is_none());
    }

    #[test]
    fn preview_uri_parts() {
        // bare id -> app's own storefront; <cc>:<id> -> that storefront
        assert_eq!(
            preview_parts("preview:123"),
            Some(("jp".into(), "123".into()))
        );
        assert_eq!(
            preview_parts("preview:us:123"),
            Some(("us".into(), "123".into()))
        );
        // a 3+-letter first segment is part of the id, not a storefront
        assert_eq!(
            preview_parts("preview:pl.u-abc"),
            Some(("jp".into(), "pl.u-abc".into()))
        );
        assert_eq!(preview_parts("file://x"), None);
        assert_eq!(preview_parts("preview:"), None);
    }

    #[test]
    fn summarize_maps_variants() {
        let v = serde_json::json!({
            "id": "1", "type": "songs",
            "attributes": {"name": "n", "artistName": "a",
                "audioVariants": ["hi-res-lossless","lossless"],
                "artwork": {"url": "https://x/{w}x{h}bb.jpg"}}
        });
        let s = summarize(&v);
        assert_eq!(s["audioVariants"][0], "hi-res-lossless");
        assert_eq!(s["artworkUrl"], "https://x/512x512bb.jpg");
    }

    #[test]
    fn ttml_strips_to_lines() {
        // shape Apple actually returns: <tt><body><div><p begin=…>text</p>…
        let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
            <p begin="00:01.00" end="00:02.00">first &amp; foremost</p>
            <p begin="00:02.00" end="00:03.00">second&lt;line&gt;</p>
            <p>third&#39;s</p>
            </div></body></tt>"#;
        let out = ttml_to_text(ttml);
        assert_eq!(out, "first & foremost\nsecond<line>\nthird's");
        assert!(ttml_to_text("").is_empty());
        assert_eq!(ttml_to_text("plain"), "plain");
        // multibyte must not corrupt (UTF-8 safe char push, not byte-as-char)
        assert_eq!(
            ttml_to_text("<tt><p>パクチーの唄</p><p>捧げる</p></tt>"),
            "パクチーの唄\n捧げる"
        );
        // <br> inside a line also breaks
        assert_eq!(ttml_to_text("<p>a<br/>b</p>"), "a\nb");
    }

    #[test]
    fn ttml_to_lines_parses_timing() {
        // real Apple shape: itunes:timing="Line", <p begin end> per line
        let ttml = r#"<tt itunes:timing="Line"><body dur="1:00"><div>
            <p begin="17.509" end="26.704" itunes:key="L1">first line</p>
            <p begin="1:06.776" end="1:10.0">second &amp; line</p>
            <p>no timing is skipped</p>
            <p begin="bad" end="1:00">bad times skipped</p>
            </div></body></tt>"#;
        let lines = ttml_to_lines(ttml);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].begin_ms, 17_509);
        assert_eq!(lines[0].end_ms, 26_704);
        assert_eq!(lines[0].text, "first line");
        assert_eq!(lines[1].begin_ms, 66_776); // m:ss.mmm
        assert_eq!(lines[1].text, "second & line");
        // word-level karaoke: spans inside a timed line carry their own times
        let word = r#"<tt itunes:timing="Word"><body><div>
            <p begin="10.0" end="12.0"><span begin="10.0" end="11.0">hel</span><span begin="11.0" end="12.0">lo</span></p>
            </div></body></tt>"#;
        let lines = ttml_to_lines(word);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0], (10_000, 11_000, "hel".to_string()));
    }
}
