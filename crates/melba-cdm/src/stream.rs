//! Streaming playback pipeline for Apple Music full tracks.
//!
//! Produces a virtual, lazily-decrypted fMP4 file (`init` + concatenated
//! media fragments) that can be handed to symphonia's isomp4 reader as a
//! seekable byte stream. Segments are fetched and decrypted only when the
//! reader touches them.
//!
//! Two key backends:
//!  * `Cdm` — licensed Widevine session; per-sample senc decryption inside
//!    the CDM (cenc for AAC, also usable for the s1/e1-guarded head of a
//!    lossless stream).
//!  * `Aes` — raw 16-byte content keys resolved from skd:// URIs by a
//!    caller-provided `KeySource` (e.g. a local Android-native wrapper
//!    service); cbcs sample decryption happens here, in plain Rust.

use crate::{Error, Res};
use std::collections::HashMap;
use std::io::Read;

// ---------- tiny box helpers ----------

fn u32b(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn u16b(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes(b[o..o + 2].try_into().unwrap())
}
fn u24(b: &[u8]) -> u32 {
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
}

/// Iterate top-level boxes of `b[range]`; yields (body_offset, total_size).
/// Stops at a box whose declared size runs past `end` (malformed input
/// must not turn into a slice panic downstream).
fn boxes<'a>(b: &'a [u8], start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> + 'a {
    let mut i = start;
    let end = end.min(b.len());
    std::iter::from_fn(move || {
        if i + 8 <= end {
            let sz = u32b(b, i) as usize;
            if sz < 8 || i + sz > end {
                return None;
            }
            let cur = i;
            i += sz;
            return Some((cur, sz));
        }
        None
    })
}

/// First child box named `name` whose header starts within [start, end).
fn child_in(b: &[u8], start: usize, end: usize, name: &[u8; 4]) -> Option<(usize, usize)> {
    boxes(b, start, end).find(|(o, _)| &b[*o + 4..*o + 8] == name)
}

// ---------- init segment: scheme, tenc, stsd rewrite ----------

/// Per-sample-entry crypto parameters (stsd can hold several enca entries;
/// fragments pick one via tfhd.sample_description_index).
#[derive(Clone)]
pub struct EntryCrypto {
    pub scheme: String,
    pub iv_size: usize,
    pub kid: [u8; 16],
    pub const_iv: Option<Vec<u8>>,
    /// tenc v1 crypt_byte_block / skip_byte_block; 0,0 = whole-region cbcs
    pub crypt: u8,
    pub skip: u8,
}

pub struct InitInfo {
    /// "cenc" or "cbcs" (schm)
    pub scheme: String,
    /// tenc: default_Per_Sample_IV_Size (8 for cenc, 0 for Apple cbcs)
    pub iv_size: usize,
    /// tenc default_KID (first protected sample entry)
    pub kid: [u8; 16],
    /// tenc constant IV for cbcs v1 (None on cenc)
    pub const_iv: Option<Vec<u8>>,
    /// codec fourcc after de-encryption (frma): "alac" / "mp4a"
    pub codec: String,
    /// sample entry: (rate, channels, bits) from the audio sample entry
    pub rate: u32,
    pub channels: u32,
    pub bits: u32,
    /// crypto params of every encrypted stsd entry, in order
    pub entries: Vec<EntryCrypto>,
}

/// Parse moov for crypto parameters and rewrite encrypted audio sample
/// entries (`enca`/`encv`) in place so a normal demuxer accepts the init:
/// the entry type becomes its `sinf/frma` original format and `sinf` is
/// removed. Returns the parsed InitInfo; `init` is modified in place.
pub fn parse_and_clean_init(init: &mut Vec<u8>) -> Res<InitInfo> {
    let b = init.clone();
    let moov = boxes(&b, 0, b.len())
        .find(|(o, _)| &b[*o + 4..*o + 8] == b"moov")
        .ok_or_else(|| Error::Parse("no moov".into()))?;
    let trak = child_in(&b, moov.0 + 8, moov.0 + moov.1, b"trak")
        .ok_or_else(|| Error::Parse("no trak".into()))?;
    let mdia = child_in(&b, trak.0 + 8, trak.0 + trak.1, b"mdia")
        .ok_or_else(|| Error::Parse("no mdia".into()))?;
    let minf = child_in(&b, mdia.0 + 8, mdia.0 + mdia.1, b"minf")
        .ok_or_else(|| Error::Parse("no minf".into()))?;
    let stbl = child_in(&b, minf.0 + 8, minf.0 + minf.1, b"stbl")
        .ok_or_else(|| Error::Parse("no stbl".into()))?;
    let stsd = child_in(&b, stbl.0 + 8, stbl.0 + stbl.1, b"stsd")
        .ok_or_else(|| Error::Parse("no stsd".into()))?;

    // walk stsd entries (fullbox: version+flags+count = 8 bytes after header)
    let (so, ssz) = stsd;
    let mut info: Option<InitInfo> = None;
    let mut entry_cryptos: Vec<EntryCrypto> = Vec::new();
    for (eo, esz) in boxes(&b, so + 16, so + ssz) {
        let etype = &b[eo + 4..eo + 8];
        if etype != b"enca" && etype != b"encv" && etype != b"encc" {
            continue;
        }
        // audio sample entry fixed fields: 8 reserved + 2 + 2 + 4 + 4(rate<<16)
        let rate = u32b(&b, eo + 32) >> 16;
        let channels = u16b(&b, eo + 24) as u32;
        let bits = u16b(&b, eo + 26) as u32;
        // child boxes of an ISO audio sample entry start at +36
        let sinf = child_in(&b, eo + 36, eo + esz, b"sinf")
            .ok_or_else(|| Error::Parse("enca without sinf".into()))?;
        let frma = child_in(&b, sinf.0 + 8, sinf.0 + sinf.1, b"frma")
            .ok_or_else(|| Error::Parse("sinf without frma".into()))?;
        let codec = String::from_utf8_lossy(&b[frma.0 + 8..frma.0 + 12]).to_string();
        let schm = child_in(&b, sinf.0 + 8, sinf.0 + sinf.1, b"schm")
            .ok_or_else(|| Error::Parse("sinf without schm".into()))?;
        let scheme = String::from_utf8_lossy(&b[schm.0 + 12..schm.0 + 16]).to_string();
        let schi = child_in(&b, sinf.0 + 8, sinf.0 + sinf.1, b"schi")
            .ok_or_else(|| Error::Parse("sinf without schi".into()))?;
        let tenc = child_in(&b, schi.0 + 8, schi.0 + schi.1, b"tenc")
            .ok_or_else(|| Error::Parse("schi without tenc".into()))?;
        let t = &b[tenc.0 + 8..tenc.0 + tenc.1];
        // spec layout after the 4B fullbox header: v0 [res8][isP][ivSize]
        // v1 [crypt|skip][isP][ivSize] — but Apple emits an extra reserved
        // byte before isProtected in both, so probe both bases: the earlier
        // one wins iff it still looks sane (isProtected is a bool, ivSize a
        // plausible byte-IV size, and 16 KID bytes fit).
        let sane =
            |o: usize| -> bool { t.len() >= o + 18 && t[o] <= 1 && matches!(t[o + 1], 0 | 8 | 16) };
        let base = if sane(6) {
            6
        } else if sane(5) {
            5
        } else {
            4
        };
        let iv_size = t[base + 1] as usize;
        // tenc v1 carries crypt_byte_block/skip_byte_block in the two bytes
        // right after the fullbox header (the "extra reserved" Apple bytes)
        let (crypt, skip) = if t[0] == 1 && base >= 6 {
            (t[base - 2], t[base - 1])
        } else {
            (0, 0)
        };
        let mut kid = [0u8; 16];
        kid.copy_from_slice(&t[base + 2..base + 18]);
        let cpos = base + 18;
        let const_iv = if iv_size == 0 && t.len() > cpos + 1 {
            let n = t[cpos] as usize;
            let n = n.min(t.len() - cpos - 1);
            Some(t[cpos + 1..cpos + 1 + n].to_vec())
        } else {
            None
        };
        entry_cryptos.push(EntryCrypto {
            scheme: scheme.clone(),
            iv_size,
            kid,
            const_iv: const_iv.clone(),
            crypt,
            skip,
        });
        if info.is_none() {
            info = Some(InitInfo {
                scheme,
                iv_size,
                kid,
                const_iv,
                codec,
                rate,
                channels,
                bits,
                entries: Vec::new(),
            });
        }
    }
    let mut info = info.ok_or_else(|| Error::Parse("no encrypted sample entry".into()))?;
    info.entries = entry_cryptos;

    // rewrite: rename each encrypted entry to its frma and strip sinf,
    // recomputing box sizes bottom-up via a rebuild of the moov tree.
    *init = rewrite_stsd(&b)?;
    Ok(info)
}

/// Rebuild the init buffer with enca→frma and sinf removed.
fn rewrite_stsd(b: &[u8]) -> Res<Vec<u8>> {
    fn rebuild(b: &[u8], off: usize, sz: usize, out: &mut Vec<u8>) -> bool {
        let mut body = Vec::new();
        let mut _changed = false;
        let name = &b[off + 4..off + 8];
        let is_container = matches!(name, b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl");
        let stsd = name == b"stsd";
        if is_container {
            for (co, cs) in boxes(b, off + 8, off + sz) {
                _changed |= rebuild(b, co, cs, &mut body);
            }
        } else if stsd {
            // symphonia rejects multi-entry stsd: keep only the first
            // (protected) entry — the alternates differ just in tenc.
            body.extend_from_slice(&b[off + 8..off + 12]); // fullbox
            body.extend_from_slice(&1u32.to_be_bytes()); // entry count = 1
            let mut kept = false;
            for (eo, esz) in boxes(b, off + 16, off + sz) {
                if kept {
                    continue;
                }
                let et = &b[eo + 4..eo + 8];
                if et == b"enca" || et == b"encv" || et == b"encc" {
                    // find frma + keep non-sinf children
                    let sinf = child_in(b, eo + 36, eo + esz, b"sinf");
                    let frma = sinf.and_then(|s| child_in(b, s.0 + 8, s.0 + s.1, b"frma"));
                    let newty = frma
                        .map(|f| b[f.0 + 8..f.0 + 12].to_vec())
                        .unwrap_or_else(|| et.to_vec());
                    let mut ebody = Vec::new();
                    ebody.extend_from_slice(&b[eo + 8..eo + 36]); // fixed fields
                    for (ko, ks) in boxes(b, eo + 36, eo + esz) {
                        if &b[ko + 4..ko + 8] != b"sinf" {
                            ebody.extend_from_slice(&b[ko..ko + ks]);
                        }
                    }
                    let esz2 = (ebody.len() + 8) as u32;
                    body.extend_from_slice(&esz2.to_be_bytes());
                    body.extend_from_slice(&newty);
                    body.extend_from_slice(&ebody);
                    _changed = true;
                    kept = true;
                } else {
                    body.extend_from_slice(&b[eo..eo + esz]);
                }
            }
        } else {
            out.extend_from_slice(&b[off..off + sz]);
            return false;
        }
        out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(&b[off + 4..off + 8]);
        out.append(&mut body);
        true
    }
    let mut out = Vec::with_capacity(b.len());
    for (o, s) in boxes(b, 0, b.len()) {
        rebuild(b, o, s, &mut out);
    }
    Ok(out)
}

// ---------- master playlist (enhanced HLS) ----------

pub struct SessionKey {
    pub id: String,
    /// skd:// URI for FairPlay-family key fetch (wrapper path)
    pub skd: Option<String>,
    /// widevine pssh (data:;base64) — alternative for CDM licensing
    pub wv: Option<String>,
}

pub struct Variant {
    pub codecs: String,
    pub uri: String,
    /// session-key ids this variant uses (from audioAssetMetadata)
    pub key_ids: Vec<String>,
}

pub struct Master {
    pub keys: HashMap<String, SessionKey>,
    pub variants: Vec<Variant>,
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = line.find(&pat)? + pat.len();
    let rest = &line[i..];
    Some(&rest[..rest.find('"')?])
}

fn attr_bare<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    // attributes seen both ways: CODECS=alac and CODECS="alac"
    for part in line.split(',') {
        if let Some(v) = part.strip_prefix(name) {
            return v.strip_prefix('=').map(|s| s.trim().trim_matches('"'));
        }
    }
    None
}

/// Parse an enhancedHls master: session keys + variant list.
pub fn parse_master(text: &str, base_url: &str) -> Res<Master> {
    let mut keys = HashMap::new();
    let mut variants = Vec::new();
    let mut meta_by_uri: HashMap<String, Vec<String>> = HashMap::new();
    let mut pending: Option<String> = None; // attrs of last EXT-X-STREAM-INF
    for line in text.lines().map(str::trim) {
        if let Some(body) = line.strip_prefix("#EXT-X-SESSION-DATA:") {
            let id = attr_bare(body, "DATA-ID").unwrap_or("");
            let val = attr(body, "VALUE").unwrap_or("");
            if id == "com.apple.hls.AudioSessionKeyInfo" {
                let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, val)
                    .map_err(|e| Error::Parse(e.to_string()))?;
                let j: serde_json::Value =
                    serde_json::from_slice(&raw).map_err(|e| Error::Parse(e.to_string()))?;
                for k in j["sessionKeyList"].as_array().into_iter().flatten() {
                    let kid = k["id"].as_str().unwrap_or("").to_string();
                    let mut sk = SessionKey {
                        id: kid.clone(),
                        skd: None,
                        wv: None,
                    };
                    for d in k["dataFormatList"].as_array().into_iter().flatten() {
                        let u = d["uri"].as_str().unwrap_or("");
                        let fmt = d["format"].as_str().unwrap_or("");
                        if u.starts_with("skd://") || fmt.contains("streamingkeydelivery") {
                            sk.skd = Some(u.to_string());
                        } else if u.contains("base64") {
                            sk.wv = Some(u.to_string());
                        }
                    }
                    if !kid.is_empty() {
                        keys.insert(kid, sk);
                    }
                }
            } else if id == "com.apple.hls.audioAssetMetadata" {
                let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, val)
                    .map_err(|e| Error::Parse(e.to_string()))?;
                let j: serde_json::Value =
                    serde_json::from_slice(&raw).map_err(|e| Error::Parse(e.to_string()))?;
                for (uri, v) in j["assets"].as_object().into_iter().flatten() {
                    let ids = v["sessionKeyIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect();
                    meta_by_uri.insert(uri.clone(), ids);
                }
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(rest.to_string());
        } else if !line.starts_with('#') && !line.is_empty() {
            if let Some(attrs) = pending.take() {
                let uri = resolve_url(base_url, line);
                let key_ids = meta_by_uri
                    .get(line)
                    .or_else(|| meta_by_uri.get(&uri))
                    .cloned()
                    .unwrap_or_default();
                variants.push(Variant {
                    codecs: attr_bare(&attrs, "CODECS").unwrap_or("").to_string(),
                    uri,
                    key_ids,
                });
            }
        }
    }
    Ok(Master { keys, variants })
}

/// Join a possibly-relative playlist URI against the playlist URL.
fn resolve_url(base: &str, rel: &str) -> String {
    if rel.starts_with("http") {
        return rel.to_string();
    }
    match base.rfind('/') {
        Some(i) => format!("{}{}", &base[..i + 1], rel),
        None => rel.to_string(),
    }
}

// ---------- variant media playlist ----------

pub struct Seg {
    pub offset: u64,
    pub size: u64,
    /// EXT-X-KEY URI in force for this segment
    pub key_uri: Option<String>,
    /// EXTINF duration seconds when present
    pub dur: f64,
}

pub struct MediaPlaylist {
    /// media file URL the byte ranges index into
    pub file_url: String,
    pub init: Vec<u8>, // fetched + cleaned init segment
    pub init_info: InitInfo,
    pub segs: Vec<Seg>,
    pub dur_ms: u64,
}

/// Parse a byte-range media playlist into `(file_url, map_range, segs,
/// total_seconds)`. Split out so unit tests can exercise it without HTTP.
type ParsedMedia = (String, (u64, u64), Vec<Seg>, f64);

fn parse_media_playlist(text: &str, playlist_url: &str) -> Res<ParsedMedia> {
    let mut file_url = String::new();
    let mut map_range = (0u64, 0u64);
    let mut segs: Vec<Seg> = Vec::new();
    let mut cur_key: Option<String> = None;
    let mut pending_dur = 0f64;
    let mut pending_range: Option<(u64, u64)> = None;
    let mut total = 0f64;
    // a pending byte-range becomes a segment the moment the next line of any
    // kind arrives (Apple fMP4 playlists index one file with no per-seg URI)
    macro_rules! flush {
        () => {
            if let Some((off, size)) = pending_range.take() {
                segs.push(Seg {
                    offset: off,
                    size,
                    key_uri: cur_key.clone(),
                    dur: pending_dur,
                });
                total += pending_dur;
            }
        };
    }
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            flush!();
            // each key rotation emits one line per KEYFORMAT; only Apple's
            // streamingkeydelivery (skd://) URIs resolve through our key
            // backends — Widevine/PlayReady data: URIs would be passed to
            // the wrapper verbatim and rejected
            let fmt = attr(rest, "KEYFORMAT").unwrap_or("");
            let u = attr(rest, "URI").unwrap_or("");
            if fmt.contains("streamingkeydelivery") || u.starts_with("skd://") {
                cur_key = attr(rest, "URI").map(String::from);
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            flush!();
            if let Some(u) = attr(rest, "URI") {
                file_url = resolve_url(playlist_url, u);
            }
            if let Some(r) = attr(rest, "BYTERANGE") {
                // BYTERANGE="length@offset" → store as (offset, len)
                let mut p = r.split('@');
                let len = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let off = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                map_range = (off, len);
            }
        } else if let Some(d) = line.strip_prefix("#EXTINF:") {
            flush!();
            pending_dur = d.trim_end_matches(',').parse().unwrap_or(0.0);
        } else if let Some(r) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            flush!();
            let mut p = r.split('@');
            let size = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            // RFC 8216 §4.3.2.2: no @offset continues right after the
            // previous segment's sub-range (first segment starts at 0)
            let next_off = segs.last().map(|s| s.offset + s.size).unwrap_or(0);
            let off = p.next().and_then(|v| v.parse().ok()).unwrap_or(next_off);
            pending_range = Some((off, size));
        } else if !line.starts_with('#') && !line.is_empty() {
            if file_url.is_empty() {
                file_url = resolve_url(playlist_url, line);
            }
            flush!();
        } else {
            flush!();
        }
    }
    flush!();
    if segs.is_empty() {
        return Err(Error::Parse("no media segments".into()));
    }
    if total == 0.0 {
        // no EXTINF (common for byte-range lists): estimate from first seg
        total = 15.0 * segs.len() as f64;
    }
    Ok((file_url, map_range, segs, total))
}

/// Fetch a variant playlist, download+clean its init map, and record the
/// key URI that applies to each byte-range segment.
pub fn open_media_playlist(http: &Http, playlist_url: &str) -> Res<MediaPlaylist> {
    let text = http.get_string(playlist_url)?;
    let (file_url, map_range, segs, total) = parse_media_playlist(&text, playlist_url)?;
    let mut init = http.get_range(&file_url, map_range.0, map_range.1)?;
    let init_info = parse_and_clean_init(&mut init)?;
    Ok(MediaPlaylist {
        file_url,
        init,
        init_info,
        segs,
        dur_ms: (total * 1000.0) as u64,
    })
}

// ---------- HTTP ----------

/// Hosts that may receive `Authorization` or `media-user-token`.
/// Playback URLs in this crate are `https://*.apple.com` and
/// `https://*.mzstatic.com` (catalog, webPlayback, HLS, segments, artwork
/// CDN). Raw IPs are rejected so a redirect cannot retarget credentials
/// at an address literal.
pub(crate) fn credential_url_ok(url: &str) -> bool {
    let Ok(request) = ureq::get(url).request_url() else {
        return false;
    };
    let parsed = request.as_url();
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port() != Some(0)
        && apple_media_host(request.host())
}

fn apple_media_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || h.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    const ROOTS: &[&str] = &["apple.com", "mzstatic.com"];
    ROOTS
        .iter()
        .any(|r| h == *r || h.ends_with(&format!(".{r}")))
}

fn ensure_credential_url(url: &str) -> crate::Res<()> {
    if credential_url_ok(url) {
        Ok(())
    } else {
        Err(crate::Error::Http(
            "refusing to send Apple credentials outside the trusted HTTPS hosts".into(),
        ))
    }
}

/// Join a redirect `Location` onto `base` and keep it only when the
/// result is still an allowlisted https URL. Anything else is `None`
/// so the caller does not request that host (credentials stay put).
fn resolve_credential_redirect(base: &str, loc: &str) -> Option<String> {
    if loc.trim().is_empty() {
        return None;
    }
    let base = ureq::get(base).request_url().ok()?;
    let next = base.as_url().join(loc.trim()).ok()?;
    credential_url_ok(next.as_str()).then(|| next.to_string())
}

fn token_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        // Keep the daemon/GUI environment proxy policy on the credentialed path.
        .try_proxy_from_env(true)
        // ureq would follow redirects and keep custom headers such as
        // `media-user-token` (it only strips `authorization` / `cookie`).
        // Follow allowlisted https targets ourselves; anything else is an error.
        .redirects(0)
        .build()
}

pub(crate) fn credentialed_get(url: &str, headers: &[(&str, &str)]) -> crate::Res<ureq::Response> {
    let mut current = url.to_string();
    for _ in 0..=5 {
        ensure_credential_url(&current)?;
        let mut req = token_agent().get(&current);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = req.call().map_err(credential_request_error)?;
        if !matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
            return Ok(resp);
        }
        let loc = resp.header("location").unwrap_or("");
        current = resolve_credential_redirect(&current, loc)
            .ok_or_else(|| crate::Error::Http("refusing untrusted credentialed redirect".into()))?;
    }
    Err(crate::Error::Http("too many redirects".into()))
}

pub(crate) fn credentialed_post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
) -> crate::Res<ureq::Response> {
    ensure_credential_url(url)?;
    let mut req = token_agent().post(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = req
        .send_json(body.clone())
        .map_err(credential_request_error)?;
    // Do not replay a credentialed POST body onto a redirect target.
    if matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
        return Err(crate::Error::Http(
            "refusing credentialed POST redirect".into(),
        ));
    }
    Ok(resp)
}

// A media URL may contain a private signature, so transport errors must not echo it.
fn credential_request_error(error: ureq::Error) -> crate::Error {
    crate::Error::Http(match error {
        ureq::Error::Status(status, _) => format!("credentialed request returned HTTP {status}"),
        ureq::Error::Transport(_) => "credentialed request failed".into(),
    })
}

#[derive(Clone)]
pub struct Http {
    pub dev_token: String,
    pub user_token: String,
    pub storefront: String,
}

impl Http {
    fn auth_headers(&self) -> (String, &str) {
        (
            format!("Bearer {}", self.dev_token),
            self.user_token.as_str(),
        )
    }
    pub fn get_string(&self, url: &str) -> Res<String> {
        let (auth, user) = self.auth_headers();
        credentialed_get(
            url,
            &[
                ("Authorization", auth.as_str()),
                ("media-user-token", user),
                // web-origin dev tokens are rejected (401) without the origin proof
                ("Origin", "https://music.apple.com"),
                ("Referer", "https://music.apple.com/"),
            ],
        )?
        .into_string()
        .map_err(|e| Error::Http(e.to_string()))
    }
    /// Exact range GET: anything but a 206 carrying precisely `len` bytes is
    /// an error — a truncated or range-ignored body would silently shift every
    /// later segment boundary.
    pub fn get_range(&self, url: &str, off: u64, len: u64) -> Res<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let end = off
            .checked_add(len - 1)
            .ok_or_else(|| Error::Http(format!("range {off}+{len}: overflow")))?;
        let (auth, user) = self.auth_headers();
        let range = format!("bytes={off}-{end}");
        let resp = credentialed_get(
            url,
            &[
                ("Authorization", auth.as_str()),
                ("media-user-token", user),
                ("Origin", "https://music.apple.com"),
                ("Referer", "https://music.apple.com/"),
                ("Range", range.as_str()),
            ],
        )
        .map_err(|e| Error::Http(format!("range {off}+{len}: {e}")))?;
        if resp.status() != 206 {
            return Err(Error::Http(format!(
                "range {off}+{len}: expected 206, got {}",
                resp.status()
            )));
        }
        let mut buf = Vec::with_capacity(len as usize);
        resp.into_reader()
            .take(len)
            .read_to_end(&mut buf)
            .map_err(|e| Error::Http(e.to_string()))?;
        if buf.len() != len as usize {
            return Err(Error::Http(format!(
                "range {off}+{len}: short body {}B",
                buf.len()
            )));
        }
        Ok(buf)
    }
    /// catalog API: extendedAssetUrls.enhancedHls for a song id.
    pub fn enhanced_hls(&self, adam_id: &str) -> Res<String> {
        let url = format!(
            // amp-api-edge is the host the web player uses; api.music.apple.com
            // silently drops extendedAssetUrls for web-origin tokens
            "https://amp-api-edge.music.apple.com/v1/catalog/{}/songs/{}?extend=extendedAssetUrls",
            self.storefront, adam_id
        );
        let j: serde_json::Value = serde_json::from_str(&self.get_string(&url)?)
            .map_err(|e| Error::Parse(e.to_string()))?;
        j["data"][0]["attributes"]["extendedAssetUrls"]["enhancedHls"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Error::Missing("no extendedAssetUrls.enhancedHls".into()))
    }
}

// ---------- key sources ----------

/// Resolves an skd:// key URI to a raw 16-byte AES-128 content key.
pub trait KeySource: Send {
    fn content_key(&mut self, adam_id: &str, skd_uri: &str) -> Res<[u8; 16]>;
}

// ---------- wrapper endpoint trust ----------

/// `true` when a non-loopback wrapper host is explicitly allowed.
///
/// Opt-in is `MELBA_WRAPPER_ALLOW_REMOTE=1` or config.json
/// `"wrapper_allow_remote": true` (XDG config dir, same file as
/// `wrapper_url`). Other schemes are still rejected: this client only
/// speaks `http`, `https`, and the wrapper-v2 `tcp` port.
pub fn wrapper_remote_opt_in() -> bool {
    if std::env::var("MELBA_WRAPPER_ALLOW_REMOTE").ok().as_deref() == Some("1") {
        return true;
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    let Some(base) = base else {
        return false;
    };
    let Ok(bytes) = std::fs::read(base.join("melba/config.json")) else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("wrapper_allow_remote").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

/// Accept `url` as a wrapper endpoint.
///
/// Without `allow_remote`: `http`/`https`/`tcp` (and bare `host:port`,
/// which is the wrapper-v2 decrypt address) only when the host is
/// loopback — `127.0.0.0/8`, `::1`, or the name `localhost`. Names are
/// not resolved, so a DNS name that happens to point at loopback is
/// still remote. With `allow_remote`, the host check is skipped.
/// `file`, `ftp`, and any other scheme are never accepted.
pub fn accept_wrapper_url(url: &str, allow_remote: bool) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("wrapper url is empty or contains whitespace".into());
    }
    if url.contains('@') {
        return Err("wrapper url must not contain userinfo".into());
    }
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else if let Some(r) = url.strip_prefix("tcp://") {
        ("tcp", r)
    } else if url.contains("://") {
        return Err("wrapper url scheme must be http, https, or tcp".into());
    } else {
        ("tcp", url)
    };
    let (host, port, extra) = split_wrapper_host(rest)?;
    if scheme == "tcp" {
        if port.is_none() {
            return Err("tcp wrapper url needs host:port".into());
        }
        if extra {
            return Err("tcp wrapper url must not have a path".into());
        }
    }
    if !allow_remote && !wrapper_host_is_loopback(&host) {
        return Err(format!(
            "wrapper host {host} is not loopback; set wrapper_allow_remote true \
             in config.json or MELBA_WRAPPER_ALLOW_REMOTE=1"
        ));
    }
    Ok(())
}

/// `wrapper_url` after the loopback / scheme check. Empty input is an error.
pub fn vetted_wrapper_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    accept_wrapper_url(url, wrapper_remote_opt_in())?;
    Ok(url.to_string())
}

fn split_wrapper_host(rest: &str) -> Result<(String, Option<u16>, bool), String> {
    if rest.is_empty() {
        return Err("wrapper url has no host".into());
    }
    let (host, after) = if let Some(h) = rest.strip_prefix('[') {
        let (h, after) = h
            .split_once(']')
            .ok_or_else(|| "wrapper url has a bad IPv6 host".to_string())?;
        if h.is_empty() {
            return Err("wrapper url has no host".into());
        }
        (h.to_string(), after)
    } else {
        let end = rest.find(['/', '?', ':']).unwrap_or(rest.len());
        let host = &rest[..end];
        if host.is_empty() {
            return Err("wrapper url has no host".into());
        }
        (host.to_string(), &rest[end..])
    };
    let (port, after_port) = if let Some(a) = after.strip_prefix(':') {
        let pend = a.find(['/', '?']).unwrap_or(a.len());
        let pstr = &a[..pend];
        if pstr.is_empty() {
            return Err("wrapper url has an empty port".into());
        }
        let port: u16 = pstr
            .parse()
            .map_err(|_| format!("wrapper url has a bad port: {pstr}"))?;
        if port == 0 {
            return Err("wrapper url port must not be 0".into());
        }
        (Some(port), &a[pend..])
    } else {
        (None, after)
    };
    Ok((host, port, !after_port.is_empty()))
}

fn wrapper_host_is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        return v6.is_loopback();
    }
    false
}

/// Local wrapper service (Android-native Apple libraries running in a
/// user-supplied sandbox) that turns skd URIs into content keys.
/// Endpoint shape: GET {base}/?adamId=<id>&uri=<urlencoded skd>
/// → JSON {"contentKey": "<hex>"} (field names vary; hex or base64 both
/// accepted here). When `base_url` already carries a path (wrapper-lite
/// exposes `/key`, e.g. `http://127.0.0.1:12340/key`) the path is kept
/// as-is instead of forcing the root route.
///
/// The upstream wrapper does not authenticate this request or the TCP
/// decrypt port (`:10020` / `:40020`). This crate does not patch that
/// binary. `accept_wrapper_url` keeps the URL on loopback unless
/// `MELBA_WRAPPER_ALLOW_REMOTE=1` or config `wrapper_allow_remote` is
/// set. Another user on the machine can still call the listening ports
/// directly; that is unchanged.
pub struct WrapperKeySource {
    pub base_url: String,
}

impl KeySource for WrapperKeySource {
    fn content_key(&mut self, adam_id: &str, skd_uri: &str) -> Res<[u8; 16]> {
        accept_wrapper_url(&self.base_url, wrapper_remote_opt_in()).map_err(Error::Http)?;
        let base = self.base_url.trim_end_matches('/');
        // path after the authority: bare `http://h:p` needs `/`, a base
        // that already ends in a route like `/key` must not gain one.
        let has_path = base
            .rsplit_once("://")
            .map(|(_, rest)| rest.contains('/'))
            .unwrap_or(false);
        let url = format!(
            "{}{}?adamId={}&uri={}",
            base,
            if has_path { "" } else { "/" },
            adam_id,
            urlenc(skd_uri)
        );
        let resp = ureq::AgentBuilder::new()
            .redirects(0)
            .build()
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .call()
            .map_err(|e| Error::Http(format!("wrapper key: {e}")))?;
        if matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
            return Err(Error::Http("wrapper key redirect refused".into()));
        }
        let j: serde_json::Value = resp.into_json().map_err(|e| Error::Parse(e.to_string()))?;
        // wrapper-v2 answers {"contentKey":…} flat; wrapper-lite wraps it
        // as {"code":0,"data":{"contentKey":…}} — take whichever is set.
        let raw = j["contentKey"]
            .as_str()
            .or_else(|| j["data"]["contentKey"].as_str())
            .ok_or_else(|| Error::Parse("wrapper reply has no contentKey".into()))?;
        let bytes = if raw.len() == 32 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            (0..16)
                .map(|i| u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap_or(0))
                .collect::<Vec<u8>>()
        } else {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw)
                .map_err(|e| Error::Parse(e.to_string()))?
        };
        bytes
            .try_into()
            .map_err(|_| Error::Parse("contentKey is not 16 bytes".into()))
    }
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for c in s.bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

// ---------- decrypt backends ----------

/// The well-known content key for Apple's shared prefetch key
/// (`skd://itunes.apple.com/P000000000/s1/e1`) — public knowledge in the
/// Apple Music tooling ecosystem (gamdl bakes the same constant); it lets
/// the first segment of any track decrypt locally with no wrapper/CDM.
const PREFETCH_KEY_URI: &str = "skd://itunes.apple.com/P000000000/s1/e1";
const PREFETCH_CONTENT_KEY: [u8; 16] = [
    0x32, 0xb8, 0xad, 0xe1, 0x76, 0x9e, 0x26, 0xb1, 0xff, 0xb8, 0x98, 0x63, 0x52, 0x79, 0x3f, 0xc6,
];

/// wrapper decrypt service (the `-D/--decrypt-port`, default 10020): a
/// per-connection binary protocol. The outer loop binds a decrypt context
/// to (adamId, skd uri) — `"0"` + the prefetch uri builds the shared
/// preshare context — and the inner loop streams `[u32 LE size][cipher]`
/// frames answered with equal-length plaintext. `size=0` returns to the
/// outer loop; a zero adam byte closes. The wrapper runs Apple's closed
/// client libs; we just frame samples for it.
pub struct WrapperTcp {
    stream: std::net::TcpStream,
    ctx: Option<(String, String)>,
}

impl WrapperTcp {
    pub fn connect(addr: &str) -> Res<Self> {
        accept_wrapper_url(addr, wrapper_remote_opt_in()).map_err(Error::Http)?;
        let dial = addr.strip_prefix("tcp://").unwrap_or(addr);
        let stream = std::net::TcpStream::connect(dial)
            .map_err(|e| Error::Http(format!("wrapper tcp {addr}: {e}")))?;
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(120)))
            .ok();
        Ok(Self { stream, ctx: None })
    }

    /// Outer-loop round: switch the decrypt context to (adam, uri).
    fn set_context(&mut self, adam: &str, uri: &str) -> Res<()> {
        use std::io::Write;
        if self.ctx.as_ref().map(|(a, u)| (a.as_str(), u.as_str())) == Some((adam, uri)) {
            return Ok(());
        }
        if self.ctx.is_some() {
            // end the previous inner loop
            self.stream
                .write_all(&0u32.to_le_bytes())
                .map_err(|e| Error::Http(format!("wrapper ctx close: {e}")))?;
        }
        if adam.len() > u8::MAX as usize || uri.len() > u8::MAX as usize {
            return Err(Error::Parse("wrapper: adam/uri too long".into()));
        }
        let mut h = Vec::with_capacity(2 + adam.len() + uri.len());
        h.push(adam.len() as u8);
        h.extend_from_slice(adam.as_bytes());
        h.push(uri.len() as u8);
        h.extend_from_slice(uri.as_bytes());
        self.stream
            .write_all(&h)
            .map_err(|e| Error::Http(format!("wrapper ctx: {e}")))?;
        self.ctx = Some((adam.to_string(), uri.to_string()));
        Ok(())
    }

    /// Batch-decrypt one fragment's samples. Each item is the concatenated
    /// cipher region truncated to a 16B boundary plus the sub-16B tail.
    /// Prefetch-keyed segments bind the `"0"`/prefetch context; everything
    /// else binds the track's adamId + the segment's skd uri.
    fn decrypt_batch(
        &mut self,
        adam_id: &str,
        skd_uri: &str,
        items: &[Vec<u8>], // aligned cipher bytes per sample
    ) -> Res<Vec<Vec<u8>>> {
        use std::io::{Read, Write};
        let adam = if skd_uri == PREFETCH_KEY_URI {
            "0"
        } else {
            adam_id
        };
        self.set_context(adam, skd_uri)?;
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            if it.is_empty() {
                out.push(Vec::new());
                continue;
            }
            if it.len() % 16 != 0 {
                return Err(Error::Parse(format!(
                    "wrapper: cipher len {} not 16-aligned",
                    it.len()
                )));
            }
            self.stream
                .write_all(&(it.len() as u32).to_le_bytes())
                .and_then(|_| self.stream.write_all(it))
                .map_err(|e| Error::Http(format!("wrapper write: {e}")))?;
            let mut p = vec![0u8; it.len()];
            self.stream
                .read_exact(&mut p)
                .map_err(|e| Error::Http(format!("wrapper read: {e}")))?;
            out.push(p);
        }
        Ok(out)
    }
}

impl Drop for WrapperTcp {
    fn drop(&mut self) {
        use std::io::Write;
        // inner-loop end (u32 0) + adamSize 0 — the server's read loop
        // returns and closes the connection
        let _ = self.stream.write_all(&[0u8; 5]);
    }
}

/// License-renewal context carried alongside a CDM session — Apple issues
/// `renew-after` (~780 s) licenses, and tracks longer than that need a
/// refresh mid-playback. Sequence mirrors MusicKit:
/// `update("renew")` → CDM emits a type-1 message → same license POST →
/// `update(response)`.
pub struct RenewCtx {
    wp: crate::WebPlayback,
    license_url: String,
    key_uri: String,
    adam_id: String,
    started: std::time::Instant,
    after_secs: u64,
}

enum Dec {
    /// licensed CDM session; samples decrypted inside the CDM
    Cdm {
        sess: crate::CdmSession,
        renew: Option<RenewCtx>,
    },
    /// raw content keys; cbcs decryption in-process
    Aes {
        src: Box<dyn KeySource>,
        adam_id: String,
        keys: HashMap<String, [u8; 16]>,
    },
    /// wrapper-v2 TCP decrypt service: cipher in, plain out
    Wrapper { conn: WrapperTcp, adam_id: String },
}

/// Per-sample crypto context handed to the decrypt backends.
struct SampleCrypto<'a> {
    kid: &'a [u8; 16],
    scheme: u32, // 1=cenc 2=cbcs
    iv: &'a [u8],
    subs: &'a [(u32, u32)],
    pattern: (u8, u8), // tenc crypt_byte_block/skip_byte_block
}

impl Dec {
    fn decrypt_sample(
        &mut self,
        key_uri: Option<&str>,
        c: &SampleCrypto<'_>,
        data: &mut [u8],
    ) -> Res<()> {
        match self {
            Dec::Cdm { sess, renew } => {
                if let Some(r) = renew {
                    // refresh ~60s before the license's renew-after boundary
                    if r.after_secs > 0 && r.started.elapsed().as_secs() + 60 >= r.after_secs {
                        if let Ok(msg) = sess.license_renewal(10_000) {
                            if let Ok((lic, after)) =
                                r.wp.acquire_license(&r.license_url, &msg, &r.key_uri, &r.adam_id)
                            {
                                if sess.update(&lic).is_ok() {
                                    r.started = std::time::Instant::now();
                                    if let Some(a) = after {
                                        r.after_secs = a;
                                    }
                                }
                            }
                        }
                    }
                }
                let out = sess.decrypt(c.kid, c.scheme, c.iv, c.subs, data)?;
                if out.len() < data.len() {
                    return Err(Error::Parse(format!(
                        "cdm returned short plaintext {}<{}",
                        out.len(),
                        data.len()
                    )));
                }
                data.copy_from_slice(&out[..data.len()]);
                Ok(())
            }
            Dec::Aes { src, adam_id, keys } => {
                let uri = key_uri.ok_or_else(|| Error::Missing("segment has no key uri".into()))?;
                let subs = if c.subs.is_empty() {
                    &[(0, data.len() as u32)]
                } else {
                    c.subs
                };
                let key = match keys.get(uri) {
                    Some(k) => *k,
                    None if uri == PREFETCH_KEY_URI => {
                        keys.insert(uri.to_string(), PREFETCH_CONTENT_KEY);
                        PREFETCH_CONTENT_KEY
                    }
                    None => {
                        let k = src.content_key(adam_id, uri)?;
                        keys.insert(uri.to_string(), k);
                        k
                    }
                };
                cbcs_decrypt(&key, c.iv, c.pattern.0, c.pattern.1, subs, data)
            }
            // wrapper decrypts whole-fragment batches in decrypt_fragment;
            // per-sample dispatch never reaches it
            Dec::Wrapper { .. } => Err(Error::Parse("wrapper per-sample dispatch".into())),
        }
    }
}

/// AES-128-CBC decrypt of each cipher subsample region. cbcs restarts the
/// CBC chain at the sample's constant IV for every cipher region.
/// Concatenate the ciphered spans described by `subs` — cbcs treats a
/// sample's enc regions as one contiguous cipher stream (gamdl sends the
/// same concat to the wrapper), so block-boundary and CBC chaining apply
/// to the concat, not to each region.
fn cipher_concat(subs: &[(u32, u32)], data: &[u8]) -> Res<Vec<u8>> {
    let mut cat = Vec::new();
    let mut pos = 0usize;
    for &(clr, enc) in subs {
        pos += clr as usize;
        let end = pos + enc as usize;
        if end > data.len() {
            return Err(Error::Parse("subsample overruns sample".into()));
        }
        cat.extend_from_slice(&data[pos..end]);
        pos = end;
    }
    Ok(cat)
}

/// Splice decrypted cipher bytes back between the clear spans. `full` is
/// the decrypted concat (plain ++ tail) and must cover every enc byte.
fn splice_regions(subs: &[(u32, u32)], full: &[u8], data: &mut [u8]) -> Res<()> {
    let enc_total: usize = subs.iter().map(|&(_, e)| e as usize).sum();
    if full.len() != enc_total {
        return Err(Error::Parse(format!(
            "decrypted length {} != enc total {enc_total}",
            full.len()
        )));
    }
    let mut pos = 0usize;
    let mut doff = 0usize;
    for &(clr, enc) in subs {
        pos += clr as usize;
        let end = pos + enc as usize;
        if end > data.len() {
            return Err(Error::Parse("subsample overruns sample".into()));
        }
        data[pos..end].copy_from_slice(&full[doff..doff + enc as usize]);
        pos = end;
        doff += enc as usize;
    }
    Ok(())
}

/// cbcs decryption over the concatenated cipher stream. `crypt`/`skip`
/// are tenc v1 pattern counts; crypt=0 means the whole stream is one
/// crypt window. Mirrors gamdl's decrypt_cbcs_pattern: each crypt window
/// is CBC-chained from the previous window's last ciphertext block, and
/// sub-16B remainders pass through untouched.
fn cbcs_decrypt(
    key: &[u8; 16],
    iv: &[u8],
    crypt: u8,
    skip: u8,
    subs: &[(u32, u32)],
    data: &mut [u8],
) -> Res<()> {
    use aes::cipher::{BlockDecrypt, KeyInit};
    let cipher = aes::Aes128::new_from_slice(key).map_err(|e| Error::Parse(e.to_string()))?;
    let mut cat = cipher_concat(subs, data)?;
    let mut prev = [0u8; 16];
    let n = iv.len().min(16);
    prev[..n].copy_from_slice(&iv[..n]);
    let crypt_bytes = if crypt == 0 {
        usize::MAX
    } else {
        crypt as usize * 16
    };
    let skip_bytes = skip as usize * 16;
    let mut pos = 0usize;
    while pos < cat.len() {
        let window = crypt_bytes.min(cat.len() - pos);
        let aligned = window & !0x0f;
        for p in (pos..pos + aligned).step_by(16) {
            let ctext: [u8; 16] = cat[p..p + 16].try_into().unwrap();
            let mut block = ctext;
            cipher.decrypt_block((&mut block).into());
            for i in 0..16 {
                block[i] ^= prev[i];
            }
            cat[p..p + 16].copy_from_slice(&block);
            prev = ctext;
        }
        // sub-16B tail of a crypt window passes through (cbcs rule)
        pos += window + skip_bytes.min(cat.len() - pos - window);
    }
    splice_regions(subs, &cat, data)
}

/// tfhd.sample_description_index (0-based into the stsd entry list; absent
/// flag or absent box => 0, i.e. entry 1).
fn tfhd_sample_desc(frag: &[u8], tfhd: Option<(usize, usize)>) -> usize {
    let Some((fo, fsz)) = tfhd else { return 0 };
    let f = &frag[fo + 8..fo + fsz];
    if f.len() < 8 {
        return 0;
    }
    let fl = u24(&f[1..4]);
    let mut o = 8usize; // version+flags(4) + track_ID(4)
    if fl & 1 != 0 {
        o += 8; // base-data-offset
    }
    if fl & 2 != 0 && o + 4 <= f.len() {
        u32b(f, o) as usize - 1
    } else {
        0
    }
}

// ---------- per-fragment sample decryption ----------

/// Parse moof of a fragment, decrypt each sample's mdat bytes in place.
fn decrypt_fragment(
    frag: &mut [u8],
    key_uri: Option<&str>,
    info: &InitInfo,
    dec: &mut Dec,
) -> Res<()> {
    let moof = boxes(frag, 0, frag.len())
        .find(|(o, _)| &frag[*o + 4..*o + 8] == b"moof")
        .ok_or_else(|| Error::Parse("no moof".into()))?;
    let traf = child_in(frag, moof.0 + 8, moof.0 + moof.1, b"traf")
        .ok_or_else(|| Error::Parse("no traf".into()))?;
    // One senc table covers every trun in this traf, in sample order.
    let mut tfhd = None;
    let mut truns = Vec::new();
    let mut senc = None;
    for (o, s) in boxes(frag, traf.0 + 8, traf.0 + traf.1) {
        match &frag[o + 4..o + 8] {
            b"tfhd" => tfhd = Some((o, s)),
            b"trun" => truns.push((o, s)),
            b"senc" => senc = Some((o, s)),
            _ => {}
        }
    }
    // sample_description_index selects which stsd entry's crypto applies
    let desc_idx = tfhd_sample_desc(frag, tfhd);
    let crypto = info
        .entries
        .get(desc_idx)
        .or_else(|| info.entries.first())
        .cloned()
        .unwrap_or_else(|| EntryCrypto {
            scheme: info.scheme.clone(),
            iv_size: info.iv_size,
            kid: info.kid,
            const_iv: info.const_iv.clone(),
            crypt: 0,
            skip: 0,
        });
    let scheme = match crypto.scheme.as_str() {
        "cbcs" => 2,
        _ => 1,
    };
    if truns.is_empty() {
        return Err(Error::Parse("no trun".into()));
    }
    let default_size = if let Some((fo, fsz)) = tfhd {
        let f = &frag[fo + 8..fo + fsz];
        if f.len() < 8 {
            return Err(Error::Parse("truncated tfhd".into()));
        }
        let fl = u24(&f[1..4]);
        let o = 8
            + usize::from(fl & 1 != 0) * 8
            + usize::from(fl & 2 != 0) * 4
            + usize::from(fl & 8 != 0) * 4;
        if fl & 0x10 != 0 {
            Some(u32::from_be_bytes(
                f.get(o..o + 4)
                    .ok_or_else(|| Error::Parse("truncated tfhd default size".into()))?
                    .try_into()
                    .unwrap(),
            ))
        } else {
            None
        }
    } else {
        None
    };
    let mut entries: Vec<(usize, u32)> = Vec::new();
    let mut next_pos = moof.0 + moof.1 + 8;
    for (to, tsz) in truns {
        let t = &frag[to + 8..to + tsz];
        if t.len() < 8 {
            return Err(Error::Parse("truncated trun".into()));
        }
        let fl = u24(&t[1..4]);
        let count = u32b(t, 4) as usize;
        let mut o = 8usize;
        let mut field = || -> Res<u32> {
            let bytes = t
                .get(o..o + 4)
                .ok_or_else(|| Error::Parse("trun field overrun".into()))?;
            o += 4;
            Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
        };
        let mut pos = if fl & 1 != 0 {
            let off = field()? as i32;
            usize::try_from(moof.0 as i128 + i128::from(off))
                .map_err(|_| Error::Parse("invalid trun data offset".into()))?
        } else {
            next_pos
        };
        if fl & 4 != 0 {
            field()?;
        }
        for _ in 0..count {
            if fl & 0x100 != 0 {
                field()?;
            }
            let size = if fl & 0x200 != 0 {
                field()?
            } else {
                default_size.ok_or_else(|| Error::Parse("missing sample size".into()))?
            };
            if size == 0 {
                return Err(Error::Parse("zero sample size".into()));
            }
            if fl & 0x400 != 0 {
                field()?;
            }
            if fl & 0x800 != 0 {
                field()?;
            }
            let end = pos
                .checked_add(size as usize)
                .filter(|end| *end <= frag.len())
                .ok_or_else(|| Error::Parse("sample overruns fragment".into()))?;
            entries.push((pos, size));
            pos = end;
        }
        next_pos = pos;
    }
    let count = entries.len();
    // senc entries
    type SencEntry = (Vec<u8>, Vec<(u32, u32)>);
    let senc_entries: Vec<SencEntry> = match senc {
        Some((so, ssz)) => {
            let s = &frag[so + 8..so + ssz];
            if s.len() < 8 {
                return Err(Error::Parse("truncated senc".into()));
            }
            let subsampled = u24(&s[1..4]) & 2 != 0;
            let scount = u32b(s, 4) as usize;
            if scount != count {
                return Err(Error::Parse("senc/trun sample count mismatch".into()));
            }
            let mut o = 8usize;
            let mut v = Vec::with_capacity(scount);
            for _ in 0..scount {
                if o + crypto.iv_size > s.len() {
                    return Err(Error::Parse("senc iv overrun".into()));
                }
                let iv = s[o..o + crypto.iv_size].to_vec();
                o += crypto.iv_size;
                let mut subs = Vec::new();
                if subsampled {
                    if o + 2 > s.len() {
                        return Err(Error::Parse("senc count overrun".into()));
                    }
                    let n = u16b(s, o) as usize;
                    o += 2;
                    for _ in 0..n {
                        if o + 6 > s.len() {
                            return Err(Error::Parse("senc subsample overrun".into()));
                        }
                        let cb = u16b(s, o) as u32;
                        let sb = u32b(s, o + 2);
                        o += 6;
                        subs.push((cb, sb));
                    }
                }
                v.push((iv, subs));
            }
            v
        }
        None => Vec::new(),
    };
    // collect every sample's position + encryption metadata first so the
    // wrapper backend can batch the whole fragment in one TCP request
    struct S {
        pos: usize,
        size: usize,
        iv: Vec<u8>,
        subs: Vec<(u32, u32)>,
    }
    let mut samples = Vec::with_capacity(count);
    for (i, &(p, size)) in entries.iter().enumerate() {
        let (iv, mut subs) = match senc_entries.get(i) {
            Some((iv, subs)) => {
                let iv = if iv.is_empty() {
                    crypto.const_iv.clone().unwrap_or_default()
                } else {
                    iv.clone()
                };
                (iv, subs.clone())
            }
            None => (crypto.const_iv.clone().unwrap_or_default(), Vec::new()),
        };
        if subs.is_empty() {
            subs.push((0, size));
        }
        samples.push(S {
            pos: p,
            size: size as usize,
            iv,
            subs,
        });
    }

    if let Dec::Wrapper { conn, adam_id } = dec {
        let uri = key_uri.ok_or_else(|| Error::Missing("segment has no key uri".into()))?;
        if crypto.crypt != 0 || crypto.skip != 0 {
            return Err(Error::Parse(
                "pattern encryption is not supported by the wrapper path".into(),
            ));
        }
        // per-sample concat of cipher regions, 16B-aligned for the wrapper;
        // samples with no full cipher block pass through unchanged (cbcs)
        let mut items: Vec<Vec<u8>> = Vec::new();
        let mut sent: Vec<usize> = Vec::new();
        let mut tails: Vec<Vec<u8>> = Vec::new();
        for (si, s) in samples.iter().enumerate() {
            // an absent subsample table means the whole sample is cipher
            let whole = [(0u32, s.size as u32)];
            let subs = if s.subs.is_empty() {
                &whole[..]
            } else {
                &s.subs
            };
            let cat = cipher_concat(subs, &frag[s.pos..s.pos + s.size])?;
            let aligned_len = cat.len() & !0x0f;
            if aligned_len == 0 {
                continue;
            }
            items.push(cat[..aligned_len].to_vec());
            tails.push(cat[aligned_len..].to_vec());
            sent.push(si);
        }
        if items.is_empty() {
            return Ok(());
        }
        let plains = conn.decrypt_batch(adam_id, uri, &items)?;
        if plains.len() != items.len() {
            return Err(Error::Parse("wrapper: count mismatch".into()));
        }
        for (k, &si) in sent.iter().enumerate() {
            let s = &samples[si];
            let whole = [(0u32, s.size as u32)];
            let subs = if s.subs.is_empty() {
                &whole[..]
            } else {
                &s.subs
            };
            let mut full = plains[k].clone();
            full.extend_from_slice(&tails[k]);
            splice_regions(subs, &full, &mut frag[s.pos..s.pos + s.size])?;
        }
        return Ok(());
    }

    for s in &samples {
        dec.decrypt_sample(
            key_uri,
            &SampleCrypto {
                kid: &crypto.kid,
                scheme,
                iv: &s.iv,
                subs: &s.subs,
                pattern: (crypto.crypt, crypto.skip),
            },
            &mut frag[s.pos..s.pos + s.size],
        )?;
    }
    Ok(())
}

// ---------- the virtual media file ----------

/// A lazily materialized, decrypted fMP4 stream. Implements byte-range
/// reads over the layout `[clean_init][frag0][frag1]…`; each fragment is
/// fetched on first touch and decrypted in place so downstream sees plain
/// isomp4.
pub struct VirtualMedia {
    http: Http,
    init_len: u64,
    layout: Vec<(u64 /*vstart*/, u64 /*vend*/, usize /*seg*/)>,
    seg_bufs: HashMap<usize, Vec<u8>>, // fetched+decrypted fragments
    raw: HashMap<usize, Vec<u8>>,      // prefetched, still-encrypted fragments
    rx: std::sync::mpsc::Receiver<(usize, Res<Vec<u8>>)>,
    tx: std::sync::mpsc::Sender<(usize, Res<Vec<u8>>)>,
    inflight: std::collections::HashSet<usize>,
    m: MediaPlaylist,
    dec: Dec,
    pos: u64,
    pub len: u64,
}

/// How many segments ahead of a first-touch read to prefetch in the
/// background. Symphonia's isomp4 probe walks every moof atom before
/// handing out packets, so a purely lazy fetch serializes ~one HTTP
/// range request per segment into startup latency; overlapping the raw
/// fetches cuts that to ~1/PREFETCH while decryption stays serialized
/// inside `load_seg` (the CDM session is single-threaded).
const PREFETCH: usize = 4;

impl VirtualMedia {
    /// Build the byte-level view over a media playlist. `dec` performs the
    /// per-sample decryption as fragments are touched.
    pub fn new(http: Http, m: MediaPlaylist, dec_kind: DecKind) -> Self {
        let init_len = m.init.len() as u64;
        let mut v = init_len;
        let layout = m
            .segs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let start = v;
                v += s.size;
                (start, v, i)
            })
            .collect();
        let len = v;
        let dec = match dec_kind {
            DecKind::Cdm { sess, renew } => Dec::Cdm { sess, renew },
            DecKind::Keys { src, adam_id } => Dec::Aes {
                src,
                adam_id,
                keys: HashMap::new(),
            },
            DecKind::Wrapper { conn, adam_id } => Dec::Wrapper { conn, adam_id },
        };
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            http,
            init_len,
            layout,
            seg_bufs: HashMap::new(),
            raw: HashMap::new(),
            rx,
            tx,
            inflight: std::collections::HashSet::new(),
            m,
            dec,
            pos: 0,
            len,
        }
    }

    /// init bytes (already rewritten to clean form)
    pub fn init(&self) -> &[u8] {
        &self.m.init
    }
    pub fn info(&self) -> &InitInfo {
        &self.m.init_info
    }
    pub fn dur_ms(&self) -> u64 {
        self.m.dur_ms
    }

    /// Drain completed background fetches into `raw`.
    fn collect_prefetch(&mut self) {
        while let Ok((i, r)) = self.rx.try_recv() {
            self.inflight.remove(&i);
            if let Ok(b) = r {
                self.raw.insert(i, b);
            }
            // errors are dropped: load_seg re-fetches synchronously,
            // which surfaces the failure at the natural error boundary.
        }
    }

    /// Kick background fetches for the next `PREFETCH` untouched segments.
    fn prefetch(&mut self, from: usize) {
        for i in from..self.m.segs.len().min(from + PREFETCH) {
            if self.seg_bufs.contains_key(&i)
                || self.raw.contains_key(&i)
                || !self.inflight.insert(i)
            {
                continue;
            }
            let http = self.http.clone();
            let url = self.m.file_url.clone();
            let (off, size) = (self.m.segs[i].offset, self.m.segs[i].size);
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let r = http.get_range(&url, off, size);
                let _ = tx.send((i, r));
            });
        }
    }

    fn load_seg(&mut self, i: usize) -> Res<()> {
        if self.seg_bufs.contains_key(&i) {
            return Ok(());
        }
        self.collect_prefetch();
        let s = &self.m.segs[i];
        let mut buf = match self.raw.remove(&i) {
            Some(b) => b,
            None => {
                // cold read or an in-flight prefetch: wait briefly for a
                // running prefetch of this segment before fetching again
                if self.inflight.contains(&i) {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
                    loop {
                        match self.rx.recv_timeout(std::time::Duration::from_millis(200)) {
                            Ok((j, r)) => {
                                self.inflight.remove(&j);
                                if let Ok(b) = r {
                                    self.raw.insert(j, b);
                                }
                                if j == i {
                                    break;
                                }
                            }
                            Err(_) => {
                                if std::time::Instant::now() > deadline {
                                    break;
                                }
                            }
                        }
                    }
                }
                match self.raw.remove(&i) {
                    Some(b) => b,
                    None => self.http.get_range(&self.m.file_url, s.offset, s.size)?,
                }
            }
        };
        let key_uri = s.key_uri.clone();
        decrypt_fragment(
            &mut buf,
            key_uri.as_deref(),
            &self.m.init_info,
            &mut self.dec,
        )?;
        self.seg_bufs.insert(i, buf);
        self.prefetch(i + 1);
        Ok(())
    }

    pub fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> Res<usize> {
        if pos >= self.len {
            return Ok(0);
        }
        if pos < self.init_len {
            let n = ((self.init_len - pos) as usize).min(buf.len());
            buf[..n].copy_from_slice(&self.m.init[pos as usize..pos as usize + n]);
            return Ok(n);
        }
        // find segment containing pos
        let li = self
            .layout
            .iter()
            .position(|(s, e, _)| pos >= *s && pos < *e)
            .ok_or_else(|| Error::Parse("pos past end".into()))?;
        let (s, _e, i) = self.layout[li];
        self.load_seg(i)?;
        let fb = &self.seg_bufs[&i];
        let off = (pos - s) as usize;
        // get_range guarantees fb.len() == seg size so off can't exceed it;
        // saturating anyway keeps a stale layout panic-free
        let n = fb.len().saturating_sub(off).min(buf.len());
        buf[..n].copy_from_slice(&fb[off..off + n]);
        Ok(n)
    }
}

pub enum DecKind {
    Cdm {
        sess: crate::CdmSession,
        renew: Option<RenewCtx>,
    },
    Keys {
        src: Box<dyn KeySource>,
        adam_id: String,
    },
    /// wrapper-v2 TCP decrypt service
    Wrapper { conn: WrapperTcp, adam_id: String },
}

/// std::io::Read over the virtual file (sequential, position-tracked).
impl Read for VirtualMedia {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.read_at(self.pos, buf) {
            Ok(n) => {
                self.pos += n as u64;
                Ok(n)
            }
            Err(e) => Err(std::io::Error::other(e.to_string())),
        }
    }
}

impl std::io::Seek for VirtualMedia {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        // i128 math so negative results are detected, not wrapped
        let new = match p {
            std::io::SeekFrom::Start(v) => v as i128,
            std::io::SeekFrom::End(v) => self.len as i128 + v as i128,
            std::io::SeekFrom::Current(v) => self.pos as i128 + v as i128,
        };
        if !(0..=u64::MAX as i128).contains(&new) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek out of range",
            ));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

// ---------- track openers ----------

/// Lossless (ALAC) path: enhancedHls master → alac variant → VirtualMedia.
/// `wrapper` selects the user-supplied backend:
/// - `tcp://host:port` or bare `host:port` — a wrapper-v2 decrypt service
///   (gamdl convention): cipher samples go over TCP, plaintext returns.
/// - `http(s)://…` — a key-extraction service (`GET {base}?adamId&uri` →
///   JSON contentKey); cbcs decryption then runs locally with AES-128.
///
/// Fails early when the master or backend is unavailable so the caller
/// can fall back to AAC.
pub fn open_alac(http: Http, adam_id: &str, wrapper: &str) -> Res<VirtualMedia> {
    let master_url = http.enhanced_hls(adam_id)?;
    let text = http.get_string(&master_url)?;
    let master = parse_master(&text, &master_url)?;
    let variant = master
        .variants
        .iter()
        .find(|v| v.codecs.contains("alac"))
        .ok_or_else(|| Error::Missing("no alac variant".into()))?;
    let m = open_media_playlist(&http, &variant.uri)?;

    let w = wrapper.trim();
    if w.is_empty() {
        return Err(Error::Missing(
            "lossless needs a FairPlay decrypt service: set wrapper_url in \
             ~/.config/melba/config.json (tcp://host:port for wrapper-v2, or \
             http://host for a key service) — Apple only issues track keys \
             to its own clients"
                .into(),
        ));
    }
    accept_wrapper_url(w, wrapper_remote_opt_in()).map_err(Error::Http)?;
    let dec_kind = if w.starts_with("http://") || w.starts_with("https://") {
        // key-extraction service: fail fast by resolving every distinct
        // skd uri now so a dead backend doesn't surface mid-song; the
        // shared prefetch key is compiled in and needs no lookup.
        let mut keys = WrapperKeySource {
            base_url: w.to_string(),
        };
        let mut seen = std::collections::HashSet::new();
        for s in &m.segs {
            if let Some(u) = &s.key_uri {
                if u != PREFETCH_KEY_URI && seen.insert(u.clone()) {
                    keys.content_key(adam_id, u)?; // validates connectivity early
                }
            }
        }
        DecKind::Keys {
            src: Box::new(keys),
            adam_id: adam_id.to_string(),
        }
    } else {
        // wrapper-v2 TCP decrypt service — connect + first fragment are
        // the connectivity proof; skd keys are never extracted, the
        // service decrypts sample ciphertext directly.
        let addr = w.strip_prefix("tcp://").unwrap_or(w);
        DecKind::Wrapper {
            conn: WrapperTcp::connect(addr)?,
            adam_id: adam_id.to_string(),
        }
    };

    let mut vm = VirtualMedia::new(http, m, dec_kind);
    // pre-load first fragment so playback starts with decryption proven
    vm.load_seg(0)?;
    Ok(vm)
}

/// Lossy AAC-256 path: webPlayback ctrp asset → Widevine license →
/// VirtualMedia decrypted inside the CDM. `cdm_so`/`storage_dir` locate
/// the Widevine module and its persistent storage.
pub fn open_aac(
    http: Http,
    adam_id: &str,
    cdm_so: &std::path::Path,
    storage_dir: &std::path::Path,
) -> Res<VirtualMedia> {
    let wp = crate::WebPlayback {
        dev_token: http.dev_token.clone(),
        user_token: http.user_token.clone(),
        storefront: http.storefront.clone(),
    };
    let tr = wp.track(adam_id)?;
    let cert = wp.widevine_cert(&tr.cert_url)?;
    let mut sess = crate::CdmSession::open(cdm_so, storage_dir)?;
    sess.set_server_certificate(&cert)?;
    let ch = sess.license_challenge(&tr.kid)?;
    let (lic, renew) = wp.acquire_license(&tr.license_url, &ch, &tr.kid_data_uri, adam_id)?;
    sess.update(&lic)?;
    if !sess.has_usable_key() {
        return Err(Error::Cdm("no usable key after license".into()));
    }
    let renew = renew.map(|a| RenewCtx {
        license_url: tr.license_url.clone(),
        key_uri: tr.kid_data_uri.clone(),
        adam_id: adam_id.to_string(),
        wp,
        started: std::time::Instant::now(),
        after_secs: a,
    });
    let m = open_media_playlist(&http, &tr.url_m3u8)?;
    let mut vm = VirtualMedia::new(http, m, DecKind::Cdm { sess, renew });
    vm.load_seg(0)?;
    Ok(vm)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- byte builders for synthetic fMP4 fixtures ----------

    fn bx(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        v.extend_from_slice(name);
        v.extend_from_slice(body);
        v
    }

    /// enca sample entry: 28B fixed audio fields + sinf(frma+schm+schi/tenc)
    fn enca_entry(
        codec: &[u8; 4],
        scheme: &[u8; 4],
        kid: &[u8; 16],
        iv_size: u8,
        const_iv: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut fixed = vec![0u8; 28];
        fixed[6] = 0;
        fixed[7] = 1; // data_reference_index = 1
        fixed[16..18].copy_from_slice(&2u16.to_be_bytes()); // channelcount
        fixed[18..20].copy_from_slice(&16u16.to_be_bytes()); // samplesize
        fixed[24..28].copy_from_slice(&(44100u32 << 16).to_be_bytes()); // samplerate
                                                                        // tenc: fullbox(4) + reserved(2) + isProtected + ivSize + KID (+constIV)
        let mut tb = vec![0u8; 4];
        tb.extend_from_slice(&[0, 0]); // apple's extra reserved pair
        tb.push(1);
        tb.push(iv_size);
        tb.extend_from_slice(kid);
        if let Some(iv) = const_iv {
            tb.push(iv.len() as u8);
            tb.extend_from_slice(iv);
        }
        let tenc = bx(b"tenc", &tb);
        let schi = bx(b"schi", &tenc);
        let schm_body = {
            let mut s = vec![0u8; 4];
            s.extend_from_slice(scheme);
            s.extend_from_slice(&0u32.to_be_bytes());
            s
        };
        let schm = bx(b"schm", &schm_body);
        let frma = bx(b"frma", codec);
        let mut sinf_body = Vec::new();
        sinf_body.extend_from_slice(&frma);
        sinf_body.extend_from_slice(&schm);
        sinf_body.extend_from_slice(&schi);
        let sinf = bx(b"sinf", &sinf_body);
        let mut e = Vec::new();
        e.extend_from_slice(&fixed);
        e.extend_from_slice(&sinf);
        bx(b"enca", &e)
    }

    fn init_with_stsd(entries: Vec<Vec<u8>>) -> Vec<u8> {
        let mut sbody = vec![0u8; 4]; // fullbox
        sbody.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for e in &entries {
            sbody.extend_from_slice(e);
        }
        let stsd = bx(b"stsd", &sbody);
        let stbl = bx(b"stbl", &stsd);
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &minf);
        let trak = bx(b"trak", &mdia);
        bx(b"moov", &trak)
    }

    #[test]
    fn clean_init_collapses_multi_entry_stsd() {
        let kid1 = [0x11u8; 16];
        let kid2 = [0x22u8; 16];
        let iv = [0xabu8; 16];
        let init = init_with_stsd(vec![
            enca_entry(b"alac", b"cbcs", &kid1, 0, Some(&iv)),
            enca_entry(b"alac", b"cbcs", &kid2, 0, Some(&iv)),
        ]);
        let mut v = init.clone();
        let info = parse_and_clean_init(&mut v).unwrap();
        assert_eq!(info.scheme, "cbcs");
        assert_eq!(info.codec, "alac");
        assert_eq!(info.rate, 44100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[0].kid, kid1);
        assert_eq!(info.entries[1].kid, kid2);
        assert_eq!(info.entries[0].const_iv.as_deref(), Some(&iv[..]));

        // rewritten output: stsd count=1, first entry renamed alac, sinf gone
        let moov = boxes(&v, 0, v.len())
            .find(|(o, _)| &v[*o + 4..*o + 8] == b"moov")
            .unwrap();
        let trak = child_in(&v, moov.0 + 8, moov.0 + moov.1, b"trak").unwrap();
        let mdia = child_in(&v, trak.0 + 8, trak.0 + trak.1, b"mdia").unwrap();
        let minf = child_in(&v, mdia.0 + 8, mdia.0 + mdia.1, b"minf").unwrap();
        let stbl = child_in(&v, minf.0 + 8, minf.0 + minf.1, b"stbl").unwrap();
        let stsd = child_in(&v, stbl.0 + 8, stbl.0 + stbl.1, b"stsd").unwrap();
        assert_eq!(u32b(&v, stsd.0 + 12), 1, "stsd must advertise 1 entry");
        let ent = stsd.0 + 16;
        assert_eq!(&v[ent + 4..ent + 8], b"alac", "entry renamed to frma type");
        assert!(
            child_in(&v, ent + 36, stsd.0 + stsd.1, b"sinf").is_none(),
            "sinf stripped"
        );
        // box sizes stay consistent end-to-end (no trailing garbage)
        let total: usize = boxes(&v, 0, v.len()).map(|(_, s)| s).sum();
        assert_eq!(total, v.len());
    }

    #[test]
    fn media_playlist_parses_key_rotation_and_ranges() {
        let pl = "#EXTM3U\n\
            #EXT-X-TARGETDURATION:15\n\
            #EXT-X-MAP:URI=\"asset.m4a\",BYTERANGE=\"829@0\"\n\
            #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k1\",KEYFORMATVERSIONS=\"1\"\n\
            #EXTINF:14.95,\n\
            #EXT-X-BYTERANGE:1500000@829\n\
            #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k2\",KEYFORMATVERSIONS=\"1\"\n\
            #EXTINF:15.05,\n\
            #EXT-X-BYTERANGE:1600000@1500829\n\
            #EXT-X-ENDLIST\n";
        let (file, map, segs, total) = parse_media_playlist(pl, "https://ex.com/v.m3u8").unwrap();
        assert_eq!(file, "https://ex.com/asset.m4a");
        assert_eq!(map, (0, 829));
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].key_uri.as_deref(), Some("skd://k1"));
        assert_eq!(segs[1].key_uri.as_deref(), Some("skd://k2"));
        assert_eq!((segs[0].offset, segs[0].size), (829, 1500000));
        assert_eq!((segs[1].offset, segs[1].size), (1500829, 1600000));
        assert!((total - 30.0).abs() < 0.01);
    }

    #[test]
    fn media_playlist_byterange_without_offset_continues() {
        // RFC 8216 §4.3.2.2: a missing @offset resumes right after the
        // previous segment's sub-range — legal playlists omit it
        let pl = "#EXTM3U\n\
            #EXT-X-TARGETDURATION:15\n\
            #EXT-X-MAP:URI=\"asset.m4a\",BYTERANGE=\"829@0\"\n\
            #EXTINF:14.95,\n\
            #EXT-X-BYTERANGE:1500000@829\n\
            #EXTINF:15.05,\n\
            #EXT-X-BYTERANGE:1600000\n\
            #EXTINF:10.0,\n\
            #EXT-X-BYTERANGE:100\n\
            #EXT-X-ENDLIST\n";
        let (_f, _m, segs, _t) = parse_media_playlist(pl, "https://ex.com/v.m3u8").unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!((segs[0].offset, segs[0].size), (829, 1500000));
        assert_eq!((segs[1].offset, segs[1].size), (1500829, 1600000));
        assert_eq!((segs[2].offset, segs[2].size), (3100829, 100));
    }

    #[test]
    fn master_parses_session_keys_and_variants() {
        // {"sessionKeyList":[{"id":"k3","dataFormatList":[
        //   {"format":"streamingkeydelivery","uri":"skd://c23"},
        //   {"format":"identity","uri":"data:;base64,AAAA"}]}]}
        let ski = "{\"sessionKeyList\":[{\"id\":\"k3\",\"dataFormatList\":[{\"format\":\"streamingkeydelivery\",\"uri\":\"skd://c23\"},{\"format\":\"identity\",\"uri\":\"data:;base64,AAAA\"}]}]}";
        let ski64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, ski);
        // {"assets":{"v.m3u8":{"sessionKeyIds":["k3"]}}}
        let meta = "{\"assets\":{\"v.m3u8\":{\"sessionKeyIds\":[\"k3\"]}}}";
        let meta64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, meta);
        let master = format!(
            "#EXTM3U\n\
             #EXT-X-SESSION-DATA:DATA-ID=com.apple.hls.AudioSessionKeyInfo,VALUE=\"{ski64}\"\n\
             #EXT-X-SESSION-DATA:DATA-ID=com.apple.hls.audioAssetMetadata,VALUE=\"{meta64}\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1411000,CODECS=\"alac\"\n\
             v.m3u8\n"
        );
        let m = parse_master(&master, "https://ex.com/m.m3u8").unwrap();
        assert_eq!(m.variants.len(), 1);
        assert_eq!(m.variants[0].codecs, "alac");
        assert_eq!(m.variants[0].uri, "https://ex.com/v.m3u8");
        assert_eq!(m.variants[0].key_ids, vec!["k3".to_string()]);
        let sk = &m.keys["k3"];
        assert_eq!(sk.skd.as_deref(), Some("skd://c23"));
        assert_eq!(sk.wv.as_deref(), Some("data:;base64,AAAA"));
    }

    #[test]
    fn tfhd_desc_index_is_zero_based() {
        // traf > tfhd(version0 flags=0x2, track=1, desc_idx=2)
        let mut tb = vec![0u8, 0, 0, 2]; // ver + flags(0x2)
        tb.extend_from_slice(&1u32.to_be_bytes());
        tb.extend_from_slice(&2u32.to_be_bytes()); // sample_description_index=2
        let tfhd = bx(b"tfhd", &tb);
        let traf = bx(b"traf", &tfhd);
        // find tfhd inside traf like decrypt_fragment does
        let t = child_in(&traf, 8, traf.len(), b"tfhd");
        assert_eq!(tfhd_sample_desc(&traf, t), 1);
        // no flag => 0
        let mut tb2 = vec![0u8, 0, 0, 0];
        tb2.extend_from_slice(&1u32.to_be_bytes());
        let traf2 = bx(b"traf", &bx(b"tfhd", &tb2));
        assert_eq!(
            tfhd_sample_desc(&traf2, child_in(&traf2, 8, traf2.len(), b"tfhd")),
            0
        );
        assert_eq!(tfhd_sample_desc(&traf2, None), 0);
    }

    /// A box whose declared size passes `boxes()` (>=8) but leaves a body
    /// too short for its fixed header fields must error, not slice-panic.
    /// Regression: trun/senc bodies of <8 bytes used to panic on `u32b(t, 4)`.
    #[test]
    fn truncated_fragment_boxes_error_not_panic() {
        struct FixedKey;
        impl KeySource for FixedKey {
            fn content_key(&mut self, _a: &str, _u: &str) -> Res<[u8; 16]> {
                Ok([0; 16])
            }
        }
        let info = InitInfo {
            scheme: "cbcs".into(),
            iv_size: 0,
            kid: [0; 16],
            const_iv: Some(vec![0u8; 16]),
            codec: "alac".into(),
            rate: 44100,
            channels: 2,
            bits: 16,
            entries: vec![],
        };
        let mut dec = Dec::Aes {
            src: Box::new(FixedKey),
            adam_id: "0".into(),
            keys: std::collections::HashMap::new(),
        };
        // trun body = 4B: too short for version+flags+count
        let mut frag = bx(b"moof", &bx(b"traf", &bx(b"trun", &[0u8; 4])));
        frag.extend_from_slice(&bx(b"mdat", &[0u8; 64]));
        let err = decrypt_fragment(&mut frag, None, &info, &mut dec).unwrap_err();
        assert!(err.to_string().contains("truncated trun"), "{err}");
        // valid empty trun, senc body = 4B
        let mut traf_body = bx(b"trun", &[0u8; 8]);
        traf_body.extend_from_slice(&bx(b"senc", &[0u8; 4]));
        let mut frag = bx(b"moof", &bx(b"traf", &traf_body));
        frag.extend_from_slice(&bx(b"mdat", &[0u8; 64]));
        let err = decrypt_fragment(&mut frag, None, &info, &mut dec).unwrap_err();
        assert!(err.to_string().contains("truncated senc"), "{err}");
    }

    /// The same samples partitioned into one or eleven runs must decode identically.
    /// Real ALAC fragments used 175 senc entries but 10x17+5 trun samples.
    #[test]
    fn fragment_decryption_keeps_every_trun_in_sample_order() {
        use aes::cipher::{BlockEncrypt, KeyInit};
        struct FixedKey;
        impl KeySource for FixedKey {
            fn content_key(&mut self, _: &str, _: &str) -> Res<[u8; 16]> {
                Ok([7; 16])
            }
        }
        let iv = [3u8; 16];
        let cipher = aes::Aes128::new_from_slice(&[7; 16]).unwrap();
        let mut init = init_with_stsd(vec![enca_entry(b"alac", b"cbcs", &[0; 16], 0, Some(&iv))]);
        let info = parse_and_clean_init(&mut init).unwrap();
        for runs in [vec![175], [vec![17; 10], vec![5]].concat()] {
            let mut sizes = Vec::new();
            let mut encrypted = Vec::new();
            let mut plain = Vec::new();
            let mut senc = 2u32.to_be_bytes().to_vec();
            senc.extend_from_slice(&175u32.to_be_bytes());
            for i in 0..175usize {
                let clear = 4 + i % 3 * 4;
                sizes.push((clear + 16) as u32);
                let prefix = vec![i as u8; clear];
                plain.extend_from_slice(&prefix);
                encrypted.extend_from_slice(&prefix);
                let block = [i as u8; 16];
                plain.extend_from_slice(&block);
                let mut block = block;
                for (b, v) in block.iter_mut().zip(iv) {
                    *b ^= v;
                }
                cipher.encrypt_block((&mut block).into());
                encrypted.extend_from_slice(&block);
                senc.extend_from_slice(&1u16.to_be_bytes());
                senc.extend_from_slice(&(clear as u16).to_be_bytes());
                senc.extend_from_slice(&16u32.to_be_bytes());
            }
            let tfhd = bx(b"tfhd", &[0, 2, 0, 0, 0, 0, 0, 1]);
            let senc = bx(b"senc", &senc);
            let moof_len =
                16 + tfhd.len() + senc.len() + runs.iter().map(|n| 20 + 4 * n).sum::<usize>();
            let mut traf = tfhd;
            traf.extend_from_slice(&senc);
            let mut sample = 0;
            let mut offset = moof_len + 8;
            for n in runs {
                let mut trun = 0x201u32.to_be_bytes().to_vec();
                trun.extend_from_slice(&(n as u32).to_be_bytes());
                trun.extend_from_slice(&(offset as u32).to_be_bytes());
                for size in &sizes[sample..sample + n] {
                    trun.extend_from_slice(&size.to_be_bytes());
                    offset += *size as usize;
                }
                sample += n;
                traf.extend_from_slice(&bx(b"trun", &trun));
            }
            let mut fragment = bx(b"moof", &bx(b"traf", &traf));
            assert_eq!(fragment.len(), moof_len);
            fragment.extend_from_slice(&bx(b"mdat", &encrypted));
            let mut dec = Dec::Aes {
                src: Box::new(FixedKey),
                adam_id: "fixture".into(),
                keys: HashMap::new(),
            };
            decrypt_fragment(&mut fragment, Some("fixture-key"), &info, &mut dec).unwrap();
            assert_eq!(
                &fragment[moof_len + 8..],
                plain,
                "every run must use its corresponding senc entries"
            );
        }
    }

    #[test]
    fn cbcs_decrypts_cbc_regions_with_constant_iv() {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let key = [7u8; 16];
        let iv = [3u8; 16];
        let cipher = aes::Aes128::new_from_slice(&key).unwrap();
        // 2 full blocks of plaintext + a trailing partial (stays as-is in cbcs)
        let plain = [0xA5u8; 40];
        let mut data = plain;
        // encrypt blocks 0..2 with CBC(iv)
        let mut prev = iv;
        for b in 0..2 {
            let mut blk: [u8; 16] = data[b * 16..b * 16 + 16].try_into().unwrap();
            for i in 0..16 {
                blk[i] ^= prev[i];
            }
            cipher.encrypt_block((&mut blk).into());
            data[b * 16..b * 16 + 16].copy_from_slice(&blk);
            prev = blk;
        }
        // subs: (4 clear, 36 cipher) — cipher region covers both blocks + 4B tail
        let subs = [(4u32, 36u32)];
        // build a data buffer with a 4-byte clear prefix
        let mut buf = vec![0xEEu8; 4];
        buf.extend_from_slice(&data[..36]);
        let before_tail = buf[36..40].to_vec();
        cbcs_decrypt(&key, &iv, 0, 0, &subs, &mut buf).unwrap();
        assert_eq!(&buf[..4], &[0xEE; 4]);
        // first 32 ciphered bytes now equal plaintext blocks
        assert_eq!(&buf[4..36], &plain[..32]);
        // trailing 4 bytes pass through untouched (cbcs partial-block rule)
        assert_eq!(&buf[36..40], &before_tail[..]);
    }

    /// Enc regions of one sample form a single CBC stream: the region
    /// boundary (20B here) does not align to blocks, and the chain carries
    /// across it. Catches per-region IV-reset bugs.
    #[test]
    fn cbcs_chains_across_subsample_regions() {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let key = [9u8; 16];
        let iv = [5u8; 16];
        let cipher = aes::Aes128::new_from_slice(&key).unwrap();
        // plaintext concat of the two enc regions: 40 bytes
        let cat_plain: Vec<u8> = (0..40u8).map(|i| i.wrapping_mul(3)).collect();
        let mut cat = cat_plain.clone();
        let mut prev = iv;
        for p in (0..32).step_by(16) {
            let mut blk: [u8; 16] = cat[p..p + 16].try_into().unwrap();
            for i in 0..16 {
                blk[i] ^= prev[i];
            }
            cipher.encrypt_block((&mut blk).into());
            cat[p..p + 16].copy_from_slice(&blk);
            prev = blk;
        }
        // splice cipher cat into the sample: 4 clear | 20 enc | 6 clear | 20 enc
        let mut data = vec![0xEEu8; 4];
        data.extend_from_slice(&cat[..20]);
        data.extend_from_slice(&[0xDDu8; 6]);
        data.extend_from_slice(&cat[20..]);
        let subs = [(4u32, 20u32), (6, 20)];
        cbcs_decrypt(&key, &iv, 0, 0, &subs, &mut data).unwrap();
        let mut expect = vec![0xEEu8; 4];
        expect.extend_from_slice(&cat_plain[..20]);
        expect.extend_from_slice(&[0xDDu8; 6]);
        expect.extend_from_slice(&cat_plain[20..]);
        assert_eq!(data, expect);
    }

    /// tenc v1 pattern encryption: crypt=1, skip=1 — each 16B crypt window
    /// is CBC-chained from the prior window's last ciphertext block; skip
    /// spans and window tails pass through.
    #[test]
    fn cbcs_pattern_windows_chain() {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let key = [2u8; 16];
        let iv = [7u8; 16];
        let cipher = aes::Aes128::new_from_slice(&key).unwrap();
        // enc span: [crypt16][skip16][crypt16][skip16][crypt16] = 80 bytes
        let plain: Vec<u8> = (0..80u8).map(|i| i ^ 0x5A).collect();
        let mut cat = plain.clone();
        let mut prev = iv;
        for w in [0usize, 32, 64] {
            let mut blk: [u8; 16] = cat[w..w + 16].try_into().unwrap();
            for i in 0..16 {
                blk[i] ^= prev[i];
            }
            cipher.encrypt_block((&mut blk).into());
            cat[w..w + 16].copy_from_slice(&blk);
            prev = blk;
        }
        let subs = [(0u32, 80u32)];
        let mut data = cat;
        cbcs_decrypt(&key, &iv, 1, 1, &subs, &mut data).unwrap();
        assert_eq!(data, plain);
    }
    #[test]
    fn wrapper_tcp_batch_roundtrip() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // outer loop: [1B adam len][adam][1B uri len][uri]
            let mut lb = [0u8; 1];
            s.read_exact(&mut lb).unwrap();
            let mut adam = vec![0u8; lb[0] as usize];
            s.read_exact(&mut adam).unwrap();
            assert_eq!(adam, b"1490256995");
            s.read_exact(&mut lb).unwrap();
            let mut uri = vec![0u8; lb[0] as usize];
            s.read_exact(&mut uri).unwrap();
            assert_eq!(uri, b"skd://k");
            // inner loop: [u32 LE size][cipher] -> equal-length plain
            loop {
                let mut sz = [0u8; 4];
                if s.read_exact(&mut sz).is_err() {
                    break;
                }
                let n = u32::from_le_bytes(sz) as usize;
                if n == 0 {
                    break;
                }
                let mut c = vec![0u8; n];
                s.read_exact(&mut c).unwrap();
                s.write_all(&c).unwrap();
            }
        });
        let mut w = WrapperTcp::connect(&format!("127.0.0.1:{port}")).unwrap();
        let items = vec![vec![1u8; 32], vec![2u8; 16]];
        let plains = w.decrypt_batch("1490256995", "skd://k", &items).unwrap();
        assert_eq!(plains, items);
    }

    #[test]
    fn credential_urls_stay_on_apple_https() {
        assert!(credential_url_ok(
            "https://amp-api-edge.music.apple.com/v1/catalog/us/songs/1"
        ));
        assert!(credential_url_ok(
            "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/webPlayback"
        ));
        assert!(credential_url_ok(
            "https://audio-ssl.itunes.apple.com/itunes-assets/AudioPreview.m3u8"
        ));
        assert!(credential_url_ok(
            "https://is1-ssl.mzstatic.com/image/thumb/Music/1/512x512bb.jpg"
        ));
        assert!(credential_url_ok("https://music.apple.com:443/us"));
        assert!(!credential_url_ok("http://music.apple.com/us"));
        assert!(!credential_url_ok("https://evil.example/x"));
        assert!(!credential_url_ok("https://apple.com.evil.example/x"));
        assert!(!credential_url_ok("https://notapple.com/x"));
        assert!(!credential_url_ok("https://music.apple.com.evil/x"));
        assert!(!credential_url_ok("https://127.0.0.1/x"));
        assert!(!credential_url_ok("https://169.254.169.254/latest"));
        assert!(!credential_url_ok("https://user:pw@music.apple.com/x"));
        assert!(!credential_url_ok("https://music.apple.com:0/x"));
    }

    #[test]
    fn credential_checks_use_the_actual_http_url_parser() {
        // The HTTP parser treats a backslash as a separator, not part of a DNS label.
        let bypass = "https://untrusted.example\\.apple.com/private?signature=private-marker";
        let request = ureq::get(bypass).request_url().unwrap();
        assert_eq!(request.host(), "untrusted.example");
        assert!(!credential_url_ok(bypass));
        let error = credentialed_get(bypass, &[("media-user-token", "fixture-only")])
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-marker"));
    }

    #[test]
    fn credential_redirects_do_not_leave_the_allowlist() {
        let base = "https://audio-ssl.itunes.apple.com/pl/master.m3u8";
        assert_eq!(
            resolve_credential_redirect(base, "https://aod.itunes.apple.com/seg.mp4"),
            Some("https://aod.itunes.apple.com/seg.mp4".into())
        );
        assert_eq!(
            resolve_credential_redirect(base, "/seg.mp4"),
            Some("https://audio-ssl.itunes.apple.com/seg.mp4".into())
        );
        assert_eq!(
            resolve_credential_redirect(base, "seg.mp4"),
            Some("https://audio-ssl.itunes.apple.com/pl/seg.mp4".into())
        );
        assert_eq!(
            resolve_credential_redirect(base, "https://is1-ssl.mzstatic.com/a"),
            Some("https://is1-ssl.mzstatic.com/a".into())
        );
        assert!(resolve_credential_redirect(base, "https://evil.example/a").is_none());
        assert!(resolve_credential_redirect(base, "http://audio-ssl.itunes.apple.com/a").is_none());
        assert!(resolve_credential_redirect(base, "//evil.example/a").is_none());
        assert_eq!(
            resolve_credential_redirect(base, "../secret"),
            Some("https://audio-ssl.itunes.apple.com/secret".into())
        );
        assert_eq!(
            resolve_credential_redirect(base, "?revision=2"),
            Some("https://audio-ssl.itunes.apple.com/pl/master.m3u8?revision=2".into())
        );
        assert!(
            resolve_credential_redirect(base, "https://evil.example\\\\.apple.com/a").is_none()
        );
    }

    #[test]
    fn wrapper_url_is_loopback_unless_opt_in() {
        let ok = [
            "tcp://127.0.0.1:10020",
            "http://127.0.0.1:40020",
            "https://127.0.0.1:443/key",
            "http://localhost:12340/key",
            "http://[::1]:40020/key",
            "127.0.0.1:10020",
            "tcp://127.1.2.3:9",
        ];
        for u in ok {
            assert!(accept_wrapper_url(u, false).is_ok(), "{u}");
        }
        assert!(accept_wrapper_url("tcp://10.1.2.3:10020", false).is_err());
        assert!(accept_wrapper_url("http://evil.example:9/key", false).is_err());
        assert!(accept_wrapper_url("http://127.0.0.1.evil.example/", false).is_err());
        assert!(accept_wrapper_url("tcp://10.1.2.3:10020", true).is_ok());
        assert!(accept_wrapper_url("https://wrapper.example/key", true).is_ok());
        assert!(accept_wrapper_url("file:///tmp/x", true).is_err());
        assert!(accept_wrapper_url("ftp://127.0.0.1/x", true).is_err());
        assert!(accept_wrapper_url("http://user@127.0.0.1/", false).is_err());
        assert!(accept_wrapper_url("tcp://127.0.0.1:10020/path", false).is_err());
        assert!(accept_wrapper_url("http://127.0.0.1:0/", false).is_err());
        assert!(WrapperTcp::connect("10.1.2.3:9").is_err());
    }
}
