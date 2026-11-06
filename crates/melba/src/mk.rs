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
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
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

/// Artwork used for an accent colour is a thumbnail; anything this large is
/// not one, and is refused rather than decoded.
const ARTWORK_CAP: usize = 8 * 1024 * 1024;

/// Only an https CDN URL may be fetched for an accent colour.
fn artwork_url_ok(url: &str) -> bool {
    url.starts_with("https://") && url.len() > 8 && !url.contains(char::is_whitespace)
}

/// `/me/ratings/songs` accepts a comma-separated id list; 50 keeps the URL
/// well inside what the service answers.
const RATINGS_BATCH: usize = 50;

/// Read-only library playlists inspected when looking for the Favorites one.
/// A miss costs two requests each, so the search must not walk the library.
const FAVORITES_CANDIDATES: usize = 6;

/// Below this a playlist says nothing about whether it is the Favorites one:
/// a two-song mix is often entirely liked by accident.
const FAVORITES_MIN_TRACKS: usize = 3;

/// A playlist counts as the account's Favorites list when essentially
/// everything it lists is liked. One stale entry (a song with no catalog id,
/// or one Apple re-issued under a new id) must not hide the whole list.
fn entirely_liked(liked: usize, total: usize) -> bool {
    total >= FAVORITES_MIN_TRACKS && liked * 10 >= total * 9
}

/// The catalog id a library playlist entry plays, preferring the id Apple
/// currently reports so a superseded entry still resolves.
fn song_catalog_id(item: &Value) -> Option<String> {
    let pp = &item["attributes"]["playParams"];
    pp["reportingId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .or_else(|| pp["catalogId"].as_str().filter(|id| !id.is_empty()))
        .map(str::to_string)
}

/// A song counts as saved when it is in the library or has been loved.
/// Apple stores those as two independent signals, and "loved without being
/// added" is exactly what the Favorites playlist is.
fn saved_flag(in_library: bool, rating: Option<i64>) -> bool {
    in_library || rating == Some(1)
}

/// The body Apple accepts for a rating write. The service refuses a bare
/// `{"value": 1}` with 400, so the envelope is part of the wire contract.
fn rating_body(value: i64) -> serde_json::Value {
    serde_json::json!({
        "type": "rating",
        "attributes": { "value": value },
    })
}

const TOKEN_PAGE: &str = "https://music.apple.com/us";

pub fn storefront() -> String {
    std::env::var("MELBA_STORE").unwrap_or_else(|_| "jp".into())
}

/// Apple Music web API client. `dev` caches the developer token in memory;
/// the on-disk cache is shared with `melbad` (`~/.cache/melba/dev-token`).
pub struct MusicKit {
    http: reqwest::Client,
    /// 0 unknown, 1 reached, 2 unreachable — set from request outcomes only,
    /// so nothing here can report an outage the machine did not have.
    reach: Arc<AtomicU8>,
    dev: Mutex<Option<String>>,
    storefront: String,
}

impl MusicKit {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            reach: Arc::new(AtomicU8::new(0)),
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
            let res = match req.query(query).send().await {
                Ok(res) => {
                    self.reach.store(1, Ordering::Relaxed);
                    res
                }
                Err(error) => {
                    // A transport failure is the only thing that means the
                    // service was not reached; an HTTP status does not.
                    self.reach.store(2, Ordering::Relaxed);
                    return Err(error.to_string());
                }
            };
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
        let res = match req.send().await {
            Ok(res) => {
                self.reach.store(1, Ordering::Relaxed);
                res
            }
            Err(error) => {
                self.reach.store(2, Ordering::Relaxed);
                return Err(error.to_string());
            }
        };
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
    ///
    /// Apple Music keeps two independent user signals for a song: library
    /// membership and a like/dislike rating (`/me/ratings/songs`, value 1).
    /// A song loved from the Apple Music app lands in the account's
    /// "Favorite Songs" playlist without joining the library, and that
    /// playlist is a `user-shared` resource whose ids are per-account, so
    /// the rating is the only portable way to read the same intent. `saved`
    /// therefore means "in the library **or** rated", and the rating read is
    /// best-effort: an account or storefront that refuses it must not take
    /// the library answer down with it.
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
            if kind == "songs" {
                let ratings = self.ratings(&ids).await;
                for (n, &i) in idx.iter().enumerate() {
                    flags[i] = saved_flag(flags[i], ratings.get(&ids[n]).copied());
                }
            }
        }
        Ok(flags)
    }

    /// Whether the last request that used the network reached the service.
    /// `None` until one has been tried.
    pub fn reachability(&self) -> Option<bool> {
        match self.reach.load(Ordering::Relaxed) {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    /// Fetch an artwork image the app already holds the URL for. Artwork is a
    /// public CDN GET: no token is attached, the scheme must be https, and the
    /// body is bounded so a hostile or broken response cannot fill memory.
    pub async fn artwork_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        if !artwork_url_ok(url) {
            return Err("artwork URL refused".into());
        }
        let response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("artwork: HTTP {}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > ARTWORK_CAP as u64)
        {
            return Err("artwork larger than the accent cap".into());
        }
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        if bytes.len() > ARTWORK_CAP {
            return Err("artwork larger than the accent cap".into());
        }
        Ok(bytes.to_vec())
    }

    /// Like/dislike value per song id, from `/me/ratings/songs`. Missing ids
    /// are unrated. Never fails the caller: a refused rating read only means
    /// the library answer stands alone.
    async fn ratings(&self, ids: &[String]) -> std::collections::HashMap<String, i64> {
        let mut out = std::collections::HashMap::new();
        for chunk in ids.chunks(RATINGS_BATCH) {
            let joined = chunk.join(",");
            let answer = self
                .me_get("/ratings/songs", &[("ids", joined.as_str())])
                .await;
            match answer {
                Ok(v) => {
                    for item in v["data"].as_array().into_iter().flatten() {
                        if let (Some(id), Some(value)) =
                            (item["id"].as_str(), item["attributes"]["value"].as_i64())
                        {
                            out.insert(id.to_string(), value);
                        }
                    }
                }
                Err(error) => log::warn!("ratings read failed: {error}"),
            }
        }
        out
    }

    /// The account's "Favorite Songs" playlist, found by behaviour rather
    /// than by name or id.
    ///
    /// Apple lists liked songs nowhere: `/me/ratings/songs` needs ids, so the
    /// only enumeration of them is that playlist, and it cannot be named: it
    /// is a `user-shared` resource whose `p.*` and `pl.u-*` ids differ per
    /// account (name matching was reverted for exactly that reason).
    ///
    /// What does identify it is what it contains. Among the library playlists
    /// the account cannot edit, the Favorites one is the only one made
    /// entirely of rated songs; editorial and personal mixes are not. Each
    /// candidate costs two requests and the search is bounded, so a refusal,
    /// an account with no such playlist, or one that keeps it past the first
    /// page of the library list simply yields `None`, and the caller keeps its
    /// library-backed view.
    pub async fn favorites_playlist(&self) -> Result<Option<String>, String> {
        let listed = self
            .me_get("/library/playlists", &[("limit", "50")])
            .await?;
        let candidates: Vec<String> = listed["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| {
                p["attributes"]["canEdit"].as_bool() == Some(false)
                    && p["attributes"]["hasCatalog"].as_bool() == Some(true)
            })
            .filter_map(|p| p["id"].as_str().map(str::to_string))
            .take(FAVORITES_CANDIDATES)
            .collect();
        for id in candidates {
            if self.playlist_is_entirely_rated(&id).await? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Whether every song a playlist lists carries a like. `None` when the
    /// playlist is too short to tell.
    async fn playlist_is_entirely_rated(&self, id: &str) -> Result<bool, String> {
        let listed = self
            .me_get(
                &format!("/library/playlists/{id}/tracks"),
                &[("limit", "100")],
            )
            .await?;
        let ids: Vec<String> = listed["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| song_catalog_id(item))
            .collect();
        if ids.len() < FAVORITES_MIN_TRACKS {
            return Ok(false);
        }
        let ratings = self.ratings(&ids).await;
        let liked = ids.iter().filter(|id| ratings.get(*id) == Some(&1)).count();
        Ok(entirely_liked(liked, ids.len()))
    }

    /// Record or clear the like/dislike rating for a song, so the account's
    /// own "Favorite Songs" list agrees with the heart this app drew.
    async fn set_rating(&self, id: &str, value: Option<i64>) -> Result<(), String> {
        match value {
            Some(value) => {
                let body = rating_body(value);
                self.me_send(
                    reqwest::Method::PUT,
                    &format!("/ratings/songs/{id}"),
                    &[],
                    Some(&body),
                )
                .await
                .map(|_| ())
            }
            None => {
                self.me_write(
                    reqwest::Method::DELETE,
                    &format!("/ratings/songs/{id}"),
                    &[],
                )
                .await
            }
        }
    }

    /// Save catalog uris to the user library — one POST per kind. Songs are
    /// also rated (value 1) so the account's "Favorite Songs" list shows the
    /// same heart this app drew; adding to the library alone leaves them out.
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
            // One heart is one rating. A bulk save (a whole playlist or a
            // multi-row selection) already lands in the library in a single
            // request, and fanning it out into one rating write per song
            // would spend the account's request budget for no visible gain:
            // the library flag alone already draws the heart filled.
            if kind == "songs"
                && let [id] = ids.as_slice()
                && let Err(error) = self.set_rating(id, Some(1)).await
            {
                log::warn!("could not rate {id}: {error}");
            }
        }
        Ok(())
    }

    /// Remove catalog uris — DELETE accepts library ids (`i.*`) only, so
    /// each uri first resolves its library copy via the catalog
    /// relationship; absent copies are already unsaved (noop).
    ///
    /// The rating is cleared first and independently: a song loved from the
    /// Apple Music app has no library copy at all, so clearing only the
    /// library membership would leave the heart on after the next read.
    pub async fn unsave(&self, uris: &[String]) -> Result<(), String> {
        for uri in uris {
            let Some((kind, id)) = apple_ref(uri) else {
                continue;
            };
            if kind == "songs"
                && let Err(error) = self.set_rating(id, None).await
            {
                log::warn!("could not clear the rating for {id}: {error}");
            }
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

    /// The Favorites playlist is identified by what it contains, because
    /// Apple gives it no stable name or id.
    #[test]
    fn a_playlist_is_the_favorites_one_only_when_it_is_essentially_all_liked() {
        assert!(entirely_liked(84, 84), "everything liked");
        assert!(
            entirely_liked(84, 85),
            "one entry Apple can no longer resolve must not hide the list"
        );
        assert!(!entirely_liked(60, 85), "a mix is not a favorites list");
        assert!(!entirely_liked(0, 85), "nothing liked");
        assert!(
            !entirely_liked(2, 2),
            "too short to tell: two liked songs is a coincidence"
        );
        assert!(entirely_liked(3, 3), "three is the floor");
    }

    /// A library playlist row names the song twice; the id Apple currently
    /// reports wins so a superseded entry still resolves to a rating.
    #[test]
    fn a_playlist_entry_prefers_the_id_apple_currently_reports() {
        let superseded = json!({"attributes": {"playParams": {
            "catalogId": "1000000001", "reportingId": "2000000002"}}});
        assert_eq!(song_catalog_id(&superseded).as_deref(), Some("2000000002"));
        let same = json!({"attributes": {"playParams": {
            "catalogId": "1000000001", "reportingId": "1000000001"}}});
        assert_eq!(song_catalog_id(&same).as_deref(), Some("1000000001"));
        let missing = json!({"attributes": {"playParams": {}}});
        assert_eq!(song_catalog_id(&missing), None);
    }

    /// The accent fetch takes only https URLs the app itself produced, so a
    /// crafted tag or a stored plain-http URL cannot make it read elsewhere.
    #[test]
    fn accent_fetches_accept_only_plain_https_urls() {
        assert!(artwork_url_ok(
            "https://is1-ssl.mzstatic.com/image/thumb/x.jpg"
        ));
        assert!(!artwork_url_ok(
            "http://is1-ssl.mzstatic.com/image/thumb/x.jpg"
        ));
        assert!(!artwork_url_ok("file:///etc/passwd"));
        assert!(!artwork_url_ok("https://"));
        assert!(!artwork_url_ok("https://host/a b.jpg"));
        assert!(!artwork_url_ok(""));
    }

    /// A song loved from the Apple Music app joins the Favorites playlist
    /// without joining the library, so the library flag alone leaves the
    /// heart hollow. The rating is the account-portable signal for it.
    #[test]
    fn a_loved_song_is_saved_even_without_library_membership() {
        assert!(!saved_flag(false, None), "unrated and absent is not saved");
        assert!(
            saved_flag(false, Some(1)),
            "loved without a library copy is saved"
        );
        assert!(saved_flag(true, None), "in the library is saved");
        assert!(saved_flag(true, Some(1)), "both signals agree");
        assert!(
            !saved_flag(false, Some(-1)),
            "a dislike is not a like even though a rating exists"
        );
    }

    /// The rating write carries Apple's own envelope, not a bare value: the
    /// service answers 400 for `{"value": 1}` and 200 for this shape, so a
    /// refactor that drops `type` breaks the write silently.
    #[test]
    fn the_rating_write_uses_the_rating_envelope() {
        let body = rating_body(1);
        assert_eq!(body["type"], "rating");
        assert_eq!(body["attributes"]["value"], 1);
        assert_eq!(rating_body(-1)["attributes"]["value"], -1);
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
