//! Apple Music catalog client for the melbad port.
//!
//! Thin reqwest port of the `melbad` daemon's `musickit` module: the same endpoints, the
//! same shared token files, the same Origin requirement. Catalog IDs are
//! Apple IDs; the model keeps `melba:`-shaped URIs internally so page
//! routing and id extraction work unchanged — the daemon boundary
//! (`daemon_worker::daemon_uri`) is the only place that translates.
//!
//! Tokens shared with `melbad`:
//!   developer token   ~/.cache/melba/dev-token   (fetched like musickit.rs)
//!   music-user-token  ~/.config/melba/music-user-token  (`melbad auth open`)

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::api::models::{
    Album, Artist, ArtistRef, ExternalIds, Image, Owner, Page, Playlist, Track, TrackCount,
};

const API: &str = "https://api.music.apple.com/v1/catalog";
const AMP_API: &str = "https://amp-api-edge.music.apple.com/v1/catalog";
const ME: &str = "https://api.music.apple.com/v1/me";
/// /v1/me on the web player's own edge — library writes 401 elsewhere.
const AMP_ME: &str = "https://amp-api-edge.music.apple.com/v1/me";
const ORIGIN: &str = "https://music.apple.com";
const TOKEN_PAGE: &str = "https://music.apple.com/us";

pub fn storefront() -> String {
    std::env::var("MELBA_STORE").unwrap_or_else(|_| "jp".into())
}

/// Apple Music web API client. `dev` caches the developer token in memory;
/// the on-disk cache is shared with `melbad` (`~/.cache/melba/dev-token`).
pub struct MusicKit {
    http: reqwest::Client,
    dev: Mutex<Option<String>>,
    storefront: String,
}

impl MusicKit {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            dev: Mutex::new(None),
            storefront: storefront(),
        }
    }

    fn dev_cache_path() -> Option<std::path::PathBuf> {
        std::env::var("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .ok()
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| format!("{h}/.cache").into())
            })
            .map(|p: std::path::PathBuf| p.join("melba").join("dev-token"))
    }

    /// `~/.config/melba/music-user-token` (XDG config dir).
    pub fn token_path() -> Option<std::path::PathBuf> {
        std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .ok()
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| format!("{h}/.config").into())
            })
            .map(|p: std::path::PathBuf| p.join("melba").join("music-user-token"))
    }

    pub fn user_token() -> Option<String> {
        std::fs::read_to_string(Self::token_path()?)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    async fn dev_token(&self) -> Result<String, String> {
        if let Some(t) = self.dev.lock().ok().and_then(|d| d.clone()) {
            return Ok(t);
        }
        let t = if let Some(cached) = Self::dev_cache_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| exp_of(s).map(|e| e > now() + 300).unwrap_or(false))
        {
            cached
        } else {
            let t = self.dev_token_fetch().await?;
            if let Some(p) = Self::dev_cache_path() {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, &t);
            }
            t
        };
        if let Ok(mut d) = self.dev.lock() {
            *d = Some(t.clone());
        }
        Ok(t)
    }

    fn invalidate_dev(&self) {
        if let Ok(mut d) = self.dev.lock() {
            *d = None;
        }
        if let Some(p) = Self::dev_cache_path() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// music.apple.com's storefront HTML links an app bundle embedding
    /// several JWTs; the catalog-valid one has iss == "AMPWebPlay".
    async fn dev_token_fetch(&self) -> Result<String, String> {
        let html = self
            .http
            .get(TOKEN_PAGE)
            .send()
            .await
            .map_err(|e| format!("token page: {e}"))?
            .text()
            .await
            .map_err(|e| format!("token body: {e}"))?;
        let js_path = extract_js_path(&html).ok_or("no /assets/index~*.js on storefront")?;
        let js = self
            .http
            .get(format!("https://music.apple.com{js_path}"))
            .send()
            .await
            .map_err(|e| format!("js bundle: {e}"))?
            .text()
            .await
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
            if let Some(payload) = cand.split('.').nth(1)
                && let Ok(bytes) = b64url(payload)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            {
                match v["iss"].as_str() {
                    Some("AMPWebPlay") => return Ok(cand),
                    Some(_) if first_jwt.is_none() => first_jwt = Some(cand),
                    _ => {}
                }
            }
        }
        first_jwt.ok_or("no developer token found in app bundle".into())
    }

    /// Catalog GET against api.music.apple.com. Retries once on 401 with a
    /// refetched developer token.
    pub async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let mut retried = false;
        loop {
            let dt = self.dev_token().await?;
            let mut req = self
                .http
                .get(format!("{API}/{}{path}", self.storefront))
                .header("Authorization", format!("Bearer {dt}"))
                .header("Origin", ORIGIN);
            if let Some(ut) = Self::user_token() {
                req = req.header("Music-User-Token", ut);
            }
            let res = req.query(query).send().await.map_err(|e| e.to_string())?;
            let status = res.status();
            let body = res.text().await.map_err(|e| e.to_string())?;
            if status.as_u16() == 401 && !retried {
                retried = true;
                self.invalidate_dev();
                continue;
            }
            if !status.is_success() {
                return Err(format!(
                    "{path}: HTTP {} {}",
                    status.as_u16(),
                    &body[..body.len().min(200)]
                ));
            }
            return serde_json::from_str(&body).map_err(|e| format!("{path}: json: {e}"));
        }
    }

    /// /v1/me/* — the library endpoints; needs the music-user-token.
    pub async fn me_get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let ut = Self::user_token()
            .ok_or("no music-user-token — sign in from Settings or `melbad auth open`")?;
        let dt = self.dev_token().await?;
        let req = self
            .http
            .get(format!("{ME}{path}"))
            .header("Authorization", format!("Bearer {dt}"))
            .header("Music-User-Token", ut)
            .header("Origin", ORIGIN);
        let res = req.query(query).send().await.map_err(|e| e.to_string())?;
        let status = res.status();
        let body = res.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            if Self::reject_expired(status) {
                return Err(format!("{path}: Apple Music sign-in expired"));
            }
            return Err(format!(
                "{path}: HTTP {} {}",
                status.as_u16(),
                &body[..body.len().min(200)]
            ));
        }
        serde_json::from_str(&body).map_err(|e| format!("{path}: json: {e}"))
    }

    /// Removes the stored user token at `path` (sign-out, expiry).
    fn drop_stored_token(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    /// A 401 on a /v1/me call means the stored user token was refused
    /// server-side: drop it so the app falls back to signed-out and offers
    /// a fresh sign-in instead of failing every personal call forever.
    fn reject_expired(status: reqwest::StatusCode) -> bool {
        if status.as_u16() != 401 {
            return false;
        }
        if let Some(path) = Self::token_path() {
            Self::drop_stored_token(&path);
        }
        true
    }

    /// POST/DELETE on /v1/me — library mutations go through amp-api-edge,
    /// the host the web player calls: api.music.apple.com returns 401 on
    /// DELETE for web-origin tokens. Bodies are not read; only status.
    async fn me_write(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
    ) -> Result<(), String> {
        self.me_send(method, path, query, None).await.map(|_| ())
    }

    /// Like `me_write` but with an optional JSON body and returning the
    /// response payload (for POST endpoints that answer with a resource).
    async fn me_send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let ut = Self::user_token()
            .ok_or("no music-user-token — sign in from Settings or `melbad auth open`")?;
        let dt = self.dev_token().await?;
        let mut req = self
            .http
            .request(method, format!("{AMP_ME}{path}"))
            .header("Authorization", format!("Bearer {dt}"))
            .header("Music-User-Token", ut)
            .header("Origin", ORIGIN)
            .query(query);
        req = match body {
            Some(b) => req.json(b),
            None => req.header("Content-Length", "0"),
        };
        let res = req.send().await.map_err(|e| e.to_string())?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if status.is_success() || status.as_u16() == 202 {
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        if Self::reject_expired(status) {
            return Err(format!("{path}: Apple Music sign-in expired"));
        }
        Err(format!(
            "{path}: HTTP {} {}",
            status.as_u16(),
            &text[..text.len().min(200)]
        ))
    }

    /// Per-uri library membership flags, batched into one catalog request
    /// per kind (`fields[<kind>]=inLibrary`). Unmapped uris report false.
    pub async fn contains(&self, uris: &[String]) -> Result<Vec<bool>, String> {
        let mut flags = vec![false; uris.len()];
        for kind in ["songs", "albums", "artists", "playlists"] {
            let idx: Vec<usize> = uris
                .iter()
                .enumerate()
                .filter(|(_, u)| apple_ref(u).map(|r| r.0) == Some(kind))
                .map(|(i, _)| i)
                .collect();
            if idx.is_empty() {
                continue;
            }
            let ids: Vec<String> = idx
                .iter()
                .map(|&i| apple_ref(&uris[i]).unwrap().1.to_string())
                .collect();
            let fields = format!("fields[{kind}]");
            let v = self
                .get(
                    &format!("/{kind}"),
                    &[
                        ("ids", ids.join(",").as_str()),
                        (fields.as_str(), "inLibrary"),
                    ],
                )
                .await?;
            let mut by_id = std::collections::HashMap::new();
            for item in v["data"].as_array().into_iter().flatten() {
                if let (Some(id), Some(flag)) = (
                    item["id"].as_str(),
                    item["attributes"]["inLibrary"].as_bool(),
                ) {
                    by_id.insert(id.to_string(), flag);
                }
            }
            for (n, &i) in idx.iter().enumerate() {
                flags[i] = *by_id.get(&ids[n]).unwrap_or(&false);
            }
        }
        Ok(flags)
    }

    /// Save catalog uris to the user library — one POST per kind.
    pub async fn save(&self, uris: &[String]) -> Result<(), String> {
        for kind in ["songs", "albums", "artists", "playlists"] {
            let ids: Vec<String> = uris
                .iter()
                .filter_map(|u| apple_ref(u).and_then(|r| (r.0 == kind).then(|| r.1.to_string())))
                .collect();
            if ids.is_empty() {
                continue;
            }
            self.me_write(
                reqwest::Method::POST,
                "/library",
                &[(format!("ids[{kind}]"), ids.join(","))],
            )
            .await?;
        }
        Ok(())
    }

    /// Remove catalog uris — DELETE accepts library ids (`i.*`) only, so
    /// each uri first resolves its library copy via the catalog
    /// relationship; absent copies are already unsaved (noop).
    pub async fn unsave(&self, uris: &[String]) -> Result<(), String> {
        for uri in uris {
            let Some((kind, id)) = apple_ref(uri) else {
                continue;
            };
            let v = self.get(&format!("/{kind}/{id}/library"), &[]).await?;
            let Some(lib) = v["data"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|d| d["id"].as_str())
            else {
                continue;
            };
            self.me_write(
                reqwest::Method::DELETE,
                &format!("/library/{kind}/{lib}"),
                &[],
            )
            .await?;
        }
        Ok(())
    }

    /// Closest Apple analog of Spotify's top-tracks: the play-history tail.
    /// heavy-rotation only yields playlists/stations; recents are the only
    /// personal song signal /v1/me exposes.
    pub async fn top_tracks(&self, offset: u32) -> Result<Page<Track>, String> {
        let off = offset.to_string();
        let v = self
            .me_get(
                "/recent/played/tracks",
                &[("limit", "25"), ("offset", off.as_str())],
            )
            .await?;
        Ok(Page {
            items: v["data"]
                .as_array()
                .map(|d| d.iter().map(song).collect())
                .unwrap_or_default(),
            total: v["meta"]["total"]
                .as_u64()
                .map(|n| n as u32)
                .unwrap_or_default(),
            limit: 25,
            offset,
            next: v["next"].as_str().map(str::to_string),
        })
    }

    /// Personalized artist stations carry `ra.<artist-id>` ids — the numeric
    /// suffix is the catalog artist id (verified: ra.573943518 → LiSA).
    pub async fn top_artists(&self) -> Result<Vec<Artist>, String> {
        let v = self.me_get("/recommendations", &[]).await?;
        let ids: Vec<String> = v["data"]
            .as_array()
            .map(|groups| {
                groups
                    .iter()
                    .flat_map(|g| {
                        g["relationships"]["contents"]["data"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                    })
                    .filter(|c| c["type"] == "stations")
                    .filter_map(|c| c["id"].as_str()?.strip_prefix("ra.").map(str::to_string))
                    .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids.join(",");
        let v = self.get("/artists", &[("ids", ids.as_str())]).await?;
        Ok(v["data"]
            .as_array()
            .map(|d| d.iter().map(artist).collect())
            .unwrap_or_default())
    }

    /// Seed fields are ignored: Apple Music recommendations are
    /// server-side personalized. Expand the first recommended playlist or
    /// album into tracks so the shelf shows songs, not a dead list.
    pub async fn recommendations(&self) -> Result<Vec<Track>, String> {
        let v = self.me_get("/recommendations", &[]).await?;
        let groups = v["data"].as_array().cloned().unwrap_or_default();
        let mut pick = None;
        for g in &groups {
            for c in g["relationships"]["contents"]["data"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let kind = c["type"].as_str().unwrap_or("");
                let id = c["id"].as_str().unwrap_or("");
                if !id.is_empty() && (kind == "playlists" || kind == "albums") {
                    pick = Some((kind.to_string(), id.to_string()));
                    break;
                }
            }
            if pick.is_some() {
                break;
            }
        }
        let Some((kind, id)) = pick else {
            return Ok(Vec::new());
        };
        let v = self
            .get(&format!("/{kind}/{id}/tracks"), &[("limit", "25")])
            .await?;
        Ok(v["data"]
            .as_array()
            .map(|d| d.iter().map(song).collect())
            .unwrap_or_default())
    }

    /// Create a library playlist. Verified live: POST answers 201 with the
    /// library-playlists resource (id p.*).
    pub async fn playlist_create(&self, name: &str, description: &str) -> Result<Playlist, String> {
        let body = serde_json::json!({
            "attributes": { "name": name, "description": description },
        });
        let v = self
            .me_send(
                reqwest::Method::POST,
                "/library/playlists",
                &[],
                Some(&body),
            )
            .await?;
        Ok(playlist(&v["data"][0]))
    }

    /// Add catalog songs to a library playlist — POST {id}/tracks with
    /// {data:[{id, type:"songs"}]} (verified 204).
    pub async fn playlist_add(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        let data: Vec<Value> = uris
            .iter()
            .filter_map(|u| apple_ref(u))
            .filter(|(kind, _)| *kind == "songs")
            .map(|(_, id)| serde_json::json!({"id": id, "type": "songs"}))
            .collect();
        if data.is_empty() {
            return Ok(());
        }
        let body = serde_json::json!({ "data": data });
        self.me_send(
            reqwest::Method::POST,
            &format!("/library/playlists/{playlist_id}/tracks"),
            &[],
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    /// Remove songs from a library playlist. The endpoint is the playlist
    /// itself (not /tracks): DELETE {pid}?ids[library-songs]={i.*}&mode=all
    /// (verified 204 — the shape took live probing to find). Track uris hold
    /// catalog ids; each must resolve to its library-song i.* id first.
    pub async fn playlist_remove(&self, playlist_id: &str, uris: &[String]) -> Result<(), String> {
        let mut lib_ids = Vec::new();
        for uri in uris {
            let Some(("songs", id)) = apple_ref(uri) else {
                continue;
            };
            let v = self.get(&format!("/songs/{id}/library"), &[]).await?;
            if let Some(lib) = v["data"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|d| d["id"].as_str())
            {
                lib_ids.push(lib.to_string());
            }
        }
        if lib_ids.is_empty() {
            return Ok(());
        }
        self.me_write(
            reqwest::Method::DELETE,
            &format!("/library/playlists/{playlist_id}"),
            &[
                ("ids[library-songs]".to_string(), lib_ids.join(",")),
                ("mode".to_string(), "all".to_string()),
            ],
        )
        .await
    }

    /// Songs already in the playlist, for the duplicate-check dialog.
    /// Compares catalog ids (playlist track entries carry playParams.catalogId).
    pub async fn playlist_duplicates(
        &self,
        playlist_id: &str,
        uris: &[String],
    ) -> Result<Vec<String>, String> {
        let v = self
            .me_get(
                &format!("/library/playlists/{playlist_id}/tracks"),
                &[("limit", "100")],
            )
            .await?;
        let have: std::collections::HashSet<String> = v["data"]
            .as_array()
            .map(|d| {
                d.iter()
                    .filter_map(|t| {
                        t["attributes"]["playParams"]["catalogId"]
                            .as_str()
                            .or_else(|| t["id"].as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(uris
            .iter()
            .filter(|u| {
                apple_ref(u)
                    .map(|(_, id)| have.contains(id))
                    .unwrap_or(false)
            })
            .cloned()
            .collect())
    }

    /// Delete a library playlist — DELETE {id} verified 204 on
    /// amp-api-edge (the public host 401s for web-origin tokens).
    pub async fn playlist_delete(&self, playlist_id: &str) -> Result<(), String> {
        self.me_write(
            reqwest::Method::DELETE,
            &format!("/library/playlists/{playlist_id}"),
            &[],
        )
        .await
    }

    /// Line-synced lyrics. api.music.apple.com denies this endpoint for
    /// public developer tokens (error 40012); amp-api-edge — the host the
    /// web player itself calls — serves it.
    pub async fn lyrics(&self, id: &str) -> Result<Option<crate::lyrics::Lyrics>, String> {
        let Some(ut) = Self::user_token() else {
            return Err("sign in for lyrics".into());
        };
        let dt = self.dev_token().await?;
        let res = self
            .http
            .get(format!("{AMP_API}/{}/songs/{id}/lyrics", self.storefront))
            .header("Authorization", format!("Bearer {dt}"))
            .header("Music-User-Token", ut)
            .header("Origin", ORIGIN)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if res.status().as_u16() == 404 {
            return Ok(None);
        }
        if Self::reject_expired(res.status()) {
            return Err("lyrics: Apple Music sign-in expired".into());
        }
        let v: Value = res.json().await.map_err(|e| e.to_string())?;
        let ttml = v["data"]
            .as_array()
            .and_then(|d| d[0]["attributes"]["ttml"].as_str())
            .or_else(|| v["data"][0]["attributes"]["ttml"].as_str())
            .or_else(|| v["data"][0]["attributes"]["ttmlLocalizations"].as_str());
        Ok(ttml.map(ttml_to_lyrics))
    }
}

// ---- TTML (line-synced lyrics) ----------------------------------------------

/// `mm:ss.mmm` / `h:mm:ss.mmm` → ms.
fn ttml_ms(t: &str) -> Option<u32> {
    let t = t.trim();
    let (hms, frac) = t.split_once('.').unwrap_or((t, ""));
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s): (u32, u32, u32) = match parts.as_slice() {
        [m, s] => (0, m.parse().ok()?, s.parse().ok()?),
        [h, m, s] => (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?),
        _ => return None,
    };
    let ms: u32 = match frac.len() {
        0 => 0,
        1..=3 => frac.parse::<u32>().ok()? * 10u32.pow(3 - frac.len() as u32),
        _ => frac[..3].parse().ok()?,
    };
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

fn ttml_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn ttml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

/// TTML → synced lyrics lines. `<p begin end>` becomes one `Line`; timed
/// `<span>` word spans are folded into their line's text.
fn ttml_to_lyrics(ttml: &str) -> crate::lyrics::Lyrics {
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
        if !tag.starts_with("p ") && !tag.starts_with("p\t") {
            continue;
        }
        let (Some(begin), Some(_end)) = (ttml_attr(tag, "begin"), ttml_attr(tag, "end")) else {
            continue;
        };
        let close = ttml[i..].find("</p>").map(|c| i + c).unwrap_or(ttml.len());
        let mut text = String::new();
        let mut j = i;
        while j < close {
            if ttml[j..].starts_with('<') {
                j += ttml[j..].find('>').map(|e| e + 1).unwrap_or(close - j);
            } else {
                let next = ttml[j..close].find('<').map(|n| j + n).unwrap_or(close);
                text.push_str(&ttml[j..next]);
                j = next;
            }
        }
        let text = ttml_unescape(text.trim());
        if !text.is_empty() {
            lines.push(crate::lyrics::Line {
                at_ms: ttml_ms(begin),
                text,
            });
        }
    }
    let synced = !lines.is_empty()
        && lines
            .iter()
            .all(|l: &crate::lyrics::Line| l.at_ms.is_some());
    crate::lyrics::Lyrics {
        lines,
        synced,
        instrumental: false,
    }
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn exp_of(jwt: &str) -> Option<u64> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = b64url(payload).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v["exp"].as_u64()
}

/// `melba:{kind}:{catalog-id}` → (apple resource kind, catalog id).
/// Internal uris carry the melba scheme; the daemon's `spotify:` input, which
/// persisted state may still hold, maps the same way.
fn apple_ref(uri: &str) -> Option<(&'static str, &str)> {
    let rest = uri
        .strip_prefix("melba:")
        .or_else(|| uri.strip_prefix("spotify:"))?;
    let (kind, id) = rest.split_once(':')?;
    let kind = match kind {
        "track" => "songs",
        "album" => "albums",
        "artist" => "artists",
        "playlist" => "playlists",
        _ => return None,
    };
    Some((kind, id))
}

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

fn str_of(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

fn u32_of(v: &Value, key: &str) -> u32 {
    v[key].as_u64().unwrap_or(0) as u32
}

/// Apple song and account images share the dimensions/crop/format template.
/// Incomplete templates produce HTTP 400 rather than a usable image.
pub(crate) fn artwork_url(template: &str, width: u32, height: u32) -> String {
    template
        .replace("{w}", &width.to_string())
        .replace("{h}", &height.to_string())
        .replace("{c}", "bb")
        .replace("{f}", "jpg")
}

fn artwork(attrs: &Value) -> Vec<Image> {
    let url = attrs["artwork"]["url"].as_str().unwrap_or_default();
    if url.is_empty() {
        return Vec::new();
    }
    vec![
        Image {
            url: artwork_url(url, 640, 640),
            width: Some(640),
            height: Some(640),
        },
        Image {
            url: artwork_url(url, 300, 300),
            width: Some(300),
            height: Some(300),
        },
        Image {
            url: artwork_url(url, 64, 64),
            width: Some(64),
            height: Some(64),
        },
    ]
}

fn genres(attrs: &Value) -> Vec<String> {
    attrs["genreNames"]
        .as_array()
        .map(|gs| {
            gs.iter()
                .filter_map(|g| g.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn artist_refs(d: &Value) -> Vec<ArtistRef> {
    let attrs = &d["attributes"];
    let name = str_of(attrs, "artistName");
    let rel_id = d["relationships"]["artists"]["data"][0]["id"]
        .as_str()
        .map(str::to_string);
    if name.is_empty() && rel_id.is_none() {
        return Vec::new();
    }
    vec![ArtistRef {
        id: rel_id.clone(),
        name,
        uri: rel_id.map(|id| format!("melba:artist:{id}")),
    }]
}

/// A catalog song object → model track. `uri` keeps the internal
/// `melba:`-shaped id so every link/page path works.
pub fn song(d: &Value) -> Track {
    let attrs = &d["attributes"];
    // library-songs carry i.* ids that nothing else resolves; the catalog id
    // lives in playParams.catalogId (verified live on /library/songs).
    // When catalogId has been superseded or revoked by Apple, the active
    // playback catalog id is supplied in playParams.reportingId. Prefer
    // reportingId so stale library items resolve to active streams, while
    // retaining the original catalogId in linked_from for equivalence.
    let pp = &attrs["playParams"];
    let id = pp["reportingId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| pp["catalogId"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| str_of(d, "id"));
    let linked_from = pp["catalogId"]
        .as_str()
        .filter(|cid| !cid.is_empty() && *cid != id)
        .map(|cid| crate::api::models::LinkedTrack {
            id: Some(cid.to_string()),
            uri: format!("melba:track:{cid}"),
        });
    let album_rel = &d["relationships"]["albums"]["data"][0];
    let album_id = str_of(album_rel, "id");
    // Prefer relationship artwork when supplied; otherwise the song's
    // attributes supply the album cover.
    let album_images = {
        let related = artwork(&album_rel["attributes"]);
        if related.is_empty() {
            artwork(attrs)
        } else {
            related
        }
    };
    Track {
        id: Some(id.clone()),
        name: str_of(attrs, "name"),
        uri: format!("melba:track:{id}"),
        duration_ms: u32_of(attrs, "durationInMillis"),
        explicit: attrs["contentRating"].as_str() == Some("explicit"),
        artists: artist_refs(d),
        images: artwork(attrs),
        album: if album_id.is_empty() {
            None
        } else {
            Some(Album {
                id: album_id.clone(),
                name: str_of(attrs, "albumName"),
                uri: format!("melba:album:{album_id}"),
                images: album_images,
                ..Album::default()
            })
        },
        track_number: u32_of(attrs, "trackNumber").into(),
        disc_number: u32_of(attrs, "discNumber").into(),
        is_playable: Some(true),
        external_ids: ExternalIds {
            isrc: Some(str_of(attrs, "isrc")).filter(|s| !s.is_empty()),
        },
        linked_from,
        ..Track::default()
    }
}

pub fn album(d: &Value) -> Album {
    let attrs = &d["attributes"];
    let id = str_of(d, "id");
    Album {
        id: id.clone(),
        name: str_of(attrs, "name"),
        uri: format!("melba:album:{id}"),
        album_type: Some("album".into()),
        total_tracks: u32_of(attrs, "trackCount").into(),
        images: artwork(attrs),
        artists: artist_refs(d),
        release_date: Some(str_of(attrs, "releaseDate")).filter(|s| !s.is_empty()),
        label: Some(str_of(attrs, "recordLabel")).filter(|s| !s.is_empty()),
        genres: genres(attrs),
        ..Album::default()
    }
}

pub fn artist(d: &Value) -> Artist {
    let attrs = &d["attributes"];
    let id = str_of(d, "id");
    Artist {
        id: id.clone(),
        name: str_of(attrs, "name"),
        uri: format!("melba:artist:{id}"),
        images: artwork(attrs),
        genres: genres(attrs),
        ..Artist::default()
    }
}

pub fn playlist(d: &Value) -> Playlist {
    let attrs = &d["attributes"];
    let id = str_of(d, "id");
    let curator = str_of(attrs, "curatorName");
    Playlist {
        id: id.clone(),
        name: str_of(attrs, "name"),
        uri: format!("melba:playlist:{id}"),
        description: attrs["description"]["standard"]
            .as_str()
            .map(str::to_string),
        images: artwork(attrs),
        owner: Owner {
            id: None,
            display_name: Some(curator).filter(|s| !s.is_empty()),
            uri: None,
        },
        public: Some(true),
        tracks: Some(TrackCount {
            total: u32_of(attrs, "trackCount"),
        }),
        ..Playlist::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn song_json() -> Value {
        json!({
            "id": "1588049004",
            "type": "songs",
            "attributes": {
                "name": "往け",
                "artistName": "LiSA",
                "albumName": "往け - Single",
                "durationInMillis": 228000,
                "trackNumber": 1,
                "discNumber": 1,
                "isrc": "JPXX01234567",
                "artwork": { "url": "https://is1-ssl.mzstatic.com/{w}x{h}bb.jpg", "width": 3000, "height": 3000 }
            },
            "relationships": {
                "artists": { "data": [{ "id": "352715211", "type": "artists" }] },
                "albums": { "data": [{ "id": "1588048900", "type": "albums" }] }
            }
        })
    }

    #[test]
    fn song_maps_apple_shape_to_track() {
        let t = song(&song_json());
        assert_eq!(t.id.as_deref(), Some("1588049004"));
        assert_eq!(t.uri, "melba:track:1588049004");
        assert_eq!(t.name, "往け");
        assert_eq!(t.duration_ms, 228000);
        assert_eq!(t.artists[0].name, "LiSA");
        assert_eq!(t.artists[0].id.as_deref(), Some("352715211"));
        assert_eq!(t.artists[0].uri.as_deref(), Some("melba:artist:352715211"));
        assert_eq!(t.album.as_ref().unwrap().id, "1588048900");
        assert_eq!(t.album.as_ref().unwrap().uri, "melba:album:1588048900");
        assert_eq!(t.track_number, Some(1));
        assert_eq!(t.disc_number, Some(1));
        assert_eq!(t.external_ids.isrc.as_deref(), Some("JPXX01234567"));
        // A track row draws its album's artwork: the song's attributes carry
        // it even when the album relationship does not, and without that
        // fallback every track cover was a placeholder.
        let album = t.album.as_ref().expect("album");
        assert!(!album.images.is_empty(), "the row must have artwork");
        assert_eq!(
            album.images[0].url,
            "https://is1-ssl.mzstatic.com/640x640bb.jpg"
        );
        assert_eq!(album.images[0].width, Some(640));
    }

    #[test]
    fn library_song_prefers_reporting_id_over_stale_catalog_id() {
        let d = json!({
            "id": "i.item_001",
            "type": "library-songs",
            "attributes": {
                "name": "Superseded Track",
                "artistName": "Artist",
                "durationInMillis": 262947,
                "playParams": {
                    "id": "i.item_001",
                    "catalogId": "1000000001",
                    "reportingId": "2000000002"
                }
            }
        });
        let t = song(&d);
        assert_eq!(t.id.as_deref(), Some("2000000002"));
        assert_eq!(t.uri, "melba:track:2000000002");
        assert_eq!(
            t.linked_from.as_ref().map(|l| l.uri.as_str()),
            Some("melba:track:1000000001")
        );
        // When catalogId equals reportingId, linked_from stays None
        let mut d_same = d.clone();
        d_same["attributes"]["playParams"]["reportingId"] = json!("1000000001");
        let t_same = song(&d_same);
        assert_eq!(t_same.id.as_deref(), Some("1000000001"));
        assert!(t_same.linked_from.is_none());
    }

    #[test]
    fn a_song_uses_its_album_relationships_own_artwork_when_apple_sends_it() {
        let mut d = song_json();
        d["relationships"]["albums"]["data"][0]["attributes"] =
            json!({ "artwork": { "url": "https://img/{w}x{h}.jpg" } });
        let t = song(&d);
        let album = t.album.as_ref().expect("album");
        assert_eq!(album.images[0].url, "https://img/640x640.jpg");
    }

    #[test]
    fn album_maps_release_date_and_count() {
        let a = album(&json!({
            "id": "1588048900",
            "attributes": {
                "name": "往け - Single",
                "artistName": "LiSA",
                "trackCount": 1,
                "releaseDate": "2021-10-15",
                "artwork": { "url": "https://img/{w}x{h}.jpg" }
            }
        }));
        assert_eq!(a.id, "1588048900");
        assert_eq!(a.uri, "melba:album:1588048900");
        assert_eq!(a.name, "往け - Single");
    }

    #[test]
    fn playlist_maps_curator_and_count() {
        let p = playlist(&json!({
            "id": "pl.abc123",
            "attributes": {
                "name": "Anime Hits",
                "trackCount": 42,
                "curatorName": "Apple Music Anime",
                "description": { "standard": "Best of anime." }
            }
        }));
        assert_eq!(p.uri, "melba:playlist:pl.abc123");
        assert_eq!(p.name, "Anime Hits");
        assert_eq!(p.tracks.unwrap().total, 42);
        assert_eq!(p.owner.display_name.as_deref(), Some("Apple Music Anime"));
        assert_eq!(p.description.as_deref(), Some("Best of anime."));
    }
}

#[cfg(test)]
mod ttml_tests {
    use super::ttml_to_lyrics;

    #[test]
    fn ttml_lines_keep_timing_and_text() {
        let ttml = r#"<tt xmlns="http://www.w3.org/2003/ns/ttml" itunes:timing="Line">
            <body><div>
            <p begin="00:12.340" end="00:15.000">First line</p>
            <p begin="00:15.100" end="00:18.000"><span begin="00:15.100">Second</span> <span>line</span> &amp; more</p>
            </div></body></tt>"#;
        let l = ttml_to_lyrics(ttml);
        assert!(l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].at_ms, Some(12340));
        assert_eq!(l.lines[0].text, "First line");
        assert_eq!(l.lines[1].at_ms, Some(15100));
        assert_eq!(l.lines[1].text, "Second line & more");
    }

    #[test]
    fn ttml_ms_parses_both_shapes() {
        assert_eq!(super::ttml_ms("01:23.456"), Some(83456));
        assert_eq!(super::ttml_ms("1:02:03.5"), Some(3723500));
        assert_eq!(super::ttml_ms("00:00"), Some(0));
        assert_eq!(super::ttml_ms("bad"), None);
    }

    #[test]
    fn reject_expired_only_fires_on_401() {
        use crate::mk::MusicKit;
        // 401 itself is exercised live — calling it here would delete the
        // developer's real token file. The other statuses must never clear.
        assert!(!MusicKit::reject_expired(reqwest::StatusCode::FORBIDDEN));
        assert!(!MusicKit::reject_expired(reqwest::StatusCode::NOT_FOUND));
        assert!(!MusicKit::reject_expired(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn drop_stored_token_removes_the_file() {
        use crate::mk::MusicKit;
        let dir = std::env::temp_dir().join(format!("melba-mk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("music-user-token");
        std::fs::write(&path, "token").unwrap();
        MusicKit::drop_stored_token(&path);
        assert!(!path.exists());
        // Missing files are fine too — sign-out must not fail on them.
        MusicKit::drop_stored_token(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn apple_ref_maps_internal_uris_to_resource_kinds() {
        use super::apple_ref;
        assert_eq!(
            apple_ref("melba:track:1588049004"),
            Some(("songs", "1588049004"))
        );
        assert_eq!(
            apple_ref("melba:album:1679278166"),
            Some(("albums", "1679278166"))
        );
        assert_eq!(
            apple_ref("melba:artist:1490256993"),
            Some(("artists", "1490256993"))
        );
        assert_eq!(
            apple_ref("melba:playlist:p.abc"),
            Some(("playlists", "p.abc"))
        );
        assert_eq!(apple_ref("melba:show:x"), None);
        assert_eq!(apple_ref("stream:1588049004"), None);
        assert_eq!(apple_ref("melba:track"), None);
    }
}
