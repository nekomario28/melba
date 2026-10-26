//! Widevine CDM host for authorized Apple Music webPlayback.
//!
//! Loads the system-installed libwidevinecdm.so (from a Chromium-family
//! browser), drives it through the documented CDM host interface, and
//! exchanges licenses with Apple's acquireWebPlaybackLicense endpoint —
//! the same flow MusicKit JS uses in browsers.
//!
//! Keys never leave the CDM; this crate only passes challenges,
//! licenses, and encrypted media through the public interface.

use std::ffi::{c_int, c_void, CString};
use std::path::Path;

#[allow(non_camel_case_types)]
type MelbaCdm = c_void;

extern "C" {
    fn melba_cdm_create(so_path: *const i8, storage_dir: *const i8) -> *mut MelbaCdm;
    fn melba_cdm_init(h: *mut MelbaCdm, timeout_ms: c_int) -> c_int;
    fn melba_cdm_set_certificate(h: *mut MelbaCdm, cert: *const u8, len: u32) -> c_int;
    fn melba_cdm_request(h: *mut MelbaCdm, init: *const u8, len: u32) -> c_int;
    fn melba_cdm_next_message(
        h: *mut MelbaCdm,
        buf: *mut u8,
        cap: u32,
        ty: *mut c_int,
        timeout_ms: c_int,
    ) -> c_int;
    fn melba_cdm_update(h: *mut MelbaCdm, lic: *const u8, len: u32) -> c_int;
    fn melba_cdm_has_usable_key(h: *mut MelbaCdm) -> c_int;
    fn melba_cdm_set_kid(h: *mut MelbaCdm, kid: *const u8, len: u32);
    fn melba_cdm_decrypt(
        h: *mut MelbaCdm,
        scheme: u32,
        iv: *const u8,
        ivlen: u32,
        subs_pairs: *const u32,
        nsubs: u32,
        data: *const u8,
        dlen: u32,
        out: *mut u8,
        outcap: u32,
    ) -> c_int;
    fn melba_cdm_destroy(h: *mut MelbaCdm);
}

#[derive(Debug)]
pub enum Error {
    Missing(String),
    Cdm(String),
    Http(String),
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Missing(m) => write!(f, "{m}"),
            Error::Cdm(m) => write!(f, "cdm: {m}"),
            Error::Http(m) => write!(f, "http: {m}"),
            Error::Parse(m) => write!(f, "parse: {m}"),
        }
    }
}
impl std::error::Error for Error {}
type Res<T> = Result<T, Error>;

pub mod stream;

/// Live Widevine CDM session holding any acquired keys.
pub struct CdmSession {
    ptr: *mut MelbaCdm,
}

// The CDM is driven only through &mut methods; the pointer itself is Send-safe
// as long as a session isn't used from two threads at once.
unsafe impl Send for CdmSession {}

/// Path about to be passed to `dlopen` as the Widevine CDM.
/// Absolute, a regular file, owned by the current user or root, not group- or
/// world-writable, and named `libwidevinecdm.so`. Checked before the
/// native load; a swap between the check and `dlopen` is still possible.
pub fn validate_cdm_path(path: &Path) -> Res<()> {
    if !path.is_absolute() {
        return Err(Error::Cdm("cdm path must be absolute".into()));
    }
    if path.file_name().and_then(|s| s.to_str()) != Some("libwidevinecdm.so") {
        return Err(Error::Cdm(
            "cdm path file name must be libwidevinecdm.so".into(),
        ));
    }
    let meta = std::fs::metadata(path).map_err(|e| Error::Cdm(format!("cdm path: {e}")))?;
    if !meta.is_file() {
        return Err(Error::Cdm("cdm path must be a regular file".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != 0 && meta.uid() != current_euid() {
            return Err(Error::Cdm(
                "cdm path must be owned by the current user or root".into(),
            ));
        }
        // File-type bits sit above 0o777; 0o022 is group or world write.
        if meta.mode() & 0o022 != 0 {
            return Err(Error::Cdm(
                "cdm path must not be group- or world-writable".into(),
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(Error::Cdm("cdm path checks are unix-only".into()))
    }
}

#[cfg(unix)]
fn current_euid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

impl CdmSession {
    /// `storage_dir` receives the CDM's persistent files; it must exist.
    pub fn open(so_path: &Path, storage_dir: &Path) -> Res<Self> {
        validate_cdm_path(so_path)?;
        let so = CString::new(so_path.to_string_lossy().as_bytes())
            .map_err(|_| Error::Missing("bad so path".into()))?;
        let dir = CString::new(storage_dir.to_string_lossy().as_bytes())
            .map_err(|_| Error::Missing("bad storage dir".into()))?;
        let ptr = unsafe { melba_cdm_create(so.as_ptr(), dir.as_ptr()) };
        if ptr.is_null() {
            return Err(Error::Cdm("failed to load CDM / create instance".into()));
        }
        let s = Self { ptr };
        if unsafe { melba_cdm_init(s.ptr, 10_000) } != 0 {
            return Err(Error::Cdm("CDM did not initialize".into()));
        }
        Ok(s)
    }

    pub fn set_server_certificate(&mut self, cert: &[u8]) -> Res<()> {
        match unsafe { melba_cdm_set_certificate(self.ptr, cert.as_ptr(), cert.len() as u32) } {
            0 => Ok(()),
            e => Err(Error::Cdm(format!("SetServerCertificate rejected ({e})"))),
        }
    }

    /// Generate a license challenge for a 16-byte content key id,
    /// building the exact pssh v0 that MusicKit generates.
    pub fn license_challenge(&mut self, kid: &[u8; 16]) -> Res<Vec<u8>> {
        let pssh = musickit_pssh(kid);
        if unsafe { melba_cdm_request(self.ptr, pssh.as_ptr(), pssh.len() as u32) } < 0 {
            return Err(Error::Cdm(
                "CreateSessionAndGenerateRequest rejected".into(),
            ));
        }
        self.take_message(15_000, &[0])
    }

    /// Pop the next queued session message of an expected type.
    /// types: 0=license-request 1=renewal 2=release 3=individualization
    pub fn take_message(&mut self, timeout_ms: i32, want: &[i32]) -> Res<Vec<u8>> {
        let mut buf = vec![0u8; 1 << 16];
        let mut ty: c_int = -1;
        loop {
            let n = unsafe {
                melba_cdm_next_message(
                    self.ptr,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut ty,
                    timeout_ms,
                )
            };
            if n == 0 {
                return Err(Error::Cdm("no session message".into()));
            }
            if n as usize > buf.len() {
                buf.resize(n as usize, 0);
                continue;
            }
            buf.truncate(n as usize);
            if want.contains(&ty) {
                return Ok(buf);
            }
        }
    }

    pub fn update(&mut self, license: &[u8]) -> Res<()> {
        match unsafe { melba_cdm_update(self.ptr, license.as_ptr(), license.len() as u32) } {
            0 => Ok(()),
            e => Err(Error::Cdm(format!("UpdateSession rejected ({e})"))),
        }
    }

    /// Block until the CDM emits a license-renewal message (type 1).
    ///
    /// Apple licenses carry `renew-after` (~780s). MusicKit's renewal
    /// sequence is: `update(b"renew")` -> CDM emits a renewal message
    /// -> POST it as a normal license request -> `update(response)`.
    /// Per-track sessions rarely need this — a 4-minute song ends well
    /// inside the window — but long content or session reuse does.
    pub fn license_renewal(&mut self, timeout_ms: i32) -> Res<Vec<u8>> {
        self.take_message(timeout_ms, &[1])
    }

    pub fn has_usable_key(&self) -> bool {
        unsafe { melba_cdm_has_usable_key(self.ptr) == 1 }
    }

    /// Decrypt one media sample (cenc=1, cbcs=2).
    /// `subs` = (clear,cipher) pairs; empty means fully encrypted.
    pub fn decrypt(
        &mut self,
        kid: &[u8; 16],
        scheme: u32,
        iv: &[u8],
        subs: &[(u32, u32)],
        data: &[u8],
    ) -> Res<Vec<u8>> {
        unsafe { melba_cdm_set_kid(self.ptr, kid.as_ptr(), kid.len() as u32) };
        let flat: Vec<u32> = subs.iter().flat_map(|(a, b)| [*a, *b]).collect();
        let mut out = vec![0u8; data.len() + 4096];
        let n = unsafe {
            melba_cdm_decrypt(
                self.ptr,
                scheme,
                iv.as_ptr(),
                iv.len() as u32,
                flat.as_ptr(),
                subs.len() as u32,
                data.as_ptr(),
                data.len() as u32,
                out.as_mut_ptr(),
                out.len() as u32,
            )
        };
        if n < 0 {
            return Err(Error::Cdm(format!("Decrypt status {}", -n - 1)));
        }
        out.truncate(n as usize);
        Ok(out)
    }
}

impl Drop for CdmSession {
    fn drop(&mut self) {
        unsafe { melba_cdm_destroy(self.ptr) }
    }
}

/// pssh v0 as generated by MusicKit: header + WV system id +
/// data = protobuf {algorithm:1, key_id:<16B>}.
pub fn musickit_pssh(kid: &[u8; 16]) -> Vec<u8> {
    const SYSID: [u8; 16] = [
        0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21,
        0xed,
    ];
    let mut v = Vec::with_capacity(52);
    v.extend_from_slice(&52u32.to_be_bytes());
    v.extend_from_slice(b"pssh");
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&SYSID);
    v.extend_from_slice(&20u32.to_be_bytes());
    v.extend_from_slice(&[0x08, 0x01, 0x12, 0x10]);
    v.extend_from_slice(kid);
    v
}

// ---------- Apple webPlayback ----------

pub struct WebPlayback {
    pub dev_token: String,
    pub user_token: String,
    pub storefront: String,
}

#[derive(Debug)]
pub struct Track {
    pub url_m3u8: String,
    pub kid: [u8; 16],
    pub kid_data_uri: String,
    pub license_url: String,
    pub cert_url: String,
    pub flavor: String,
}

#[derive(Debug)]
pub struct MediaSegment {
    pub size: u32,
    pub offset: u64, // absolute file offset
}

impl WebPlayback {
    fn post(&self, url: &str, body: &serde_json::Value, user_hdr: &str) -> Res<ureq::Response> {
        let auth = format!("Bearer {}", self.dev_token);
        stream::credentialed_post_json(
            url,
            &[
                ("Authorization", auth.as_str()),
                (user_hdr, self.user_token.as_str()),
                ("X-Apple-Store-Front", self.storefront.as_str()),
                ("Origin", "https://music.apple.com"),
                ("Content-Type", "application/json"),
                ("Accept", "application/json"),
            ],
            body,
        )
    }

    /// POST webPlayback, pick the preferred ctrp (Widevine cenc) flavor,
    /// and fetch that flavor's playlist + key id.
    pub fn track(&self, adam_id: &str) -> Res<Track> {
        let resp: serde_json::Value = self
            .post(
                "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/webPlayback",
                &serde_json::json!({"salableAdamId": adam_id}),
                "media-user-token",
            )?
            .into_json()
            .map_err(|e| Error::Parse(e.to_string()))?;
        let song = &resp["songList"][0];
        let license_url = song["hls-key-server-url"]
            .as_str()
            .ok_or_else(|| Error::Parse("no hls-key-server-url".into()))?
            .to_string();
        let cert_url = song["widevine-cert-url"]
            .as_str()
            .ok_or_else(|| Error::Parse("no widevine-cert-url".into()))?
            .to_string();
        let assets = song["assets"]
            .as_array()
            .ok_or_else(|| Error::Parse("no assets".into()))?;
        // prefer highest-bitrate ctrp (cenc/Widevine) flavor
        let mut pick: Option<&serde_json::Value> = None;
        let mut best = 0u64;
        for a in assets {
            let f = a["flavor"].as_str().unwrap_or("");
            if !f.contains("ctrp") {
                continue;
            }
            let br = a["metadata"]["bitRate"].as_u64().unwrap_or(0);
            if br >= best {
                best = br;
                pick = Some(a);
            }
        }
        let a = pick.ok_or_else(|| Error::Parse("no ctrp asset".into()))?;
        let m3u8_url = a["URL"]
            .as_str()
            .ok_or_else(|| Error::Parse("asset has no URL".into()))?
            .to_string();
        let flavor = a["flavor"].as_str().unwrap_or("").to_string();

        // fetch the playlist, read EXT-X-KEY URI (data:;base64,<KID>)
        let pl = self
            .get(&m3u8_url)?
            .into_string()
            .map_err(|e| Error::Http(e.to_string()))?;
        let key_uri = pl
            .lines()
            .find(|l| l.starts_with("#EXT-X-KEY"))
            .and_then(|l| l.split("URI=\"").nth(1))
            .and_then(|s| s.split('"').next())
            .ok_or_else(|| Error::Parse("no EXT-X-KEY uri".into()))?
            .to_string();
        let b64 = key_uri
            .strip_prefix("data:;base64,")
            .ok_or_else(|| Error::Parse(format!("unexpected key uri {key_uri}")))?;
        let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .map_err(|e| Error::Parse(e.to_string()))?;
        let kid: [u8; 16] = raw
            .try_into()
            .map_err(|_| Error::Parse("KID is not 16 bytes".into()))?;

        Ok(Track {
            url_m3u8: m3u8_url,
            kid,
            kid_data_uri: key_uri,
            license_url,
            cert_url,
            flavor,
        })
    }

    pub fn get(&self, url: &str) -> Res<ureq::Response> {
        let auth = format!("Bearer {}", self.dev_token);
        stream::credentialed_get(
            url,
            &[
                ("Authorization", auth.as_str()),
                ("media-user-token", self.user_token.as_str()),
            ],
        )
    }

    /// Fetch the Widevine service certificate.
    pub fn widevine_cert(&self, cert_url: &str) -> Res<Vec<u8>> {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut self.get(cert_url)?.into_reader(), &mut buf)
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(buf)
    }

    /// Exchange a CDM challenge for a license (MusicKit request shape).
    /// Returns (license_bytes, renew_after_secs).
    pub fn acquire_license(
        &self,
        license_url: &str,
        challenge: &[u8],
        key_uri: &str,
        adam_id: &str,
    ) -> Res<(Vec<u8>, Option<u64>)> {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, challenge);
        let body = serde_json::json!({
            "challenge": b64,
            "uri": key_uri,
            "key-system": "com.widevine.alpha",
            "adamId": adam_id,
            "isLibrary": false,
            "user-initiated": true,
        });
        let auth = format!("Bearer {}", self.dev_token);
        let resp = stream::credentialed_post_json(
            license_url,
            &[
                ("Authorization", auth.as_str()),
                ("X-Apple-Music-User-Token", self.user_token.as_str()),
                ("X-Apple-Renewal", "true"),
                ("X-Apple-Store-Front", self.storefront.as_str()),
                ("Origin", "https://music.apple.com"),
                ("Content-Type", "application/json"),
                ("Accept", "application/json"),
            ],
            &body,
        )?;
        let j: serde_json::Value = resp.into_json().map_err(|e| Error::Parse(e.to_string()))?;
        if j.get("status").and_then(|s| s.as_i64()).unwrap_or(0) != 0 || j.get("license").is_none()
        {
            return Err(Error::Http(format!("license error: {j}")));
        }
        let lic = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            j["license"]
                .as_str()
                .ok_or_else(|| Error::Parse("license not a string".into()))?,
        )
        .map_err(|e| Error::Parse(e.to_string()))?;
        Ok((lic, j["renew-after"].as_u64()))
    }
}

/// Parsed HLS media playlist for a ctrp track (fMP4 byte-range segments).
pub struct Playlist {
    pub map_uri: String,
    pub map_range: (u64, u64),
    pub segments: Vec<MediaSegment>,
}

pub fn parse_playlist(text: &str) -> Res<Playlist> {
    let mut map_uri = None;
    let mut map_range = (0, 0);
    let mut segments = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            for attr in rest.split(',') {
                if let Some(u) = attr.strip_prefix("URI=\"") {
                    map_uri = Some(u.trim_end_matches('"').to_string());
                } else if let Some(r) = attr.strip_prefix("BYTERANGE=\"") {
                    let r = r.trim_end_matches('"');
                    let mut p = r.split('@');
                    map_range = (
                        p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                        p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    );
                }
            }
        } else if let Some(r) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let mut p = r.split('@');
            let size = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            // RFC 8216 §4.3.2.2: no @offset continues right after the
            // previous segment's sub-range (first segment starts at 0)
            let next_off = segments
                .last()
                .map(|s: &MediaSegment| s.offset + s.size as u64)
                .unwrap_or(0);
            let off = p.next().and_then(|v| v.parse().ok()).unwrap_or(next_off);
            segments.push(MediaSegment { size, offset: off });
        }
    }
    Ok(Playlist {
        map_uri: map_uri.ok_or_else(|| Error::Parse("no EXT-X-MAP".into()))?,
        map_range,
        segments,
    })
}

pub mod fmp4 {
    //! Minimal moof parser: per-sample IVs (senc) + sample sizes (trun).
    use super::{Error, Res};

    pub struct Sample {
        pub iv: Vec<u8>,
        pub subs: Vec<(u32, u32)>,
        pub data_off: u64, // offset inside the fragment
        pub size: u32,
    }

    fn u32b(b: &[u8], o: usize) -> Res<u32> {
        let end = o
            .checked_add(4)
            .ok_or_else(|| Error::Parse("u32 offset overflow".into()))?;
        let s = b
            .get(o..end)
            .ok_or_else(|| Error::Parse("u32 read out of range".into()))?;
        Ok(u32::from_be_bytes(s.try_into().unwrap()))
    }
    fn u16b(b: &[u8], o: usize) -> Res<u16> {
        let end = o
            .checked_add(2)
            .ok_or_else(|| Error::Parse("u16 offset overflow".into()))?;
        let s = b
            .get(o..end)
            .ok_or_else(|| Error::Parse("u16 read out of range".into()))?;
        Ok(u16::from_be_bytes(s.try_into().unwrap()))
    }

    /// Iterate ISO boxes inside `range`. A returned box is guaranteed to
    /// satisfy `i + sz <= min(end, b.len())` — callers slice on that.
    fn find_box(b: &[u8], range: (usize, usize), want: &[u8; 4]) -> Res<Option<(usize, usize)>> {
        let (mut i, end) = range;
        while i + 8 <= end && i + 8 <= b.len() {
            let sz = u32b(b, i)? as usize;
            let box_end = i
                .checked_add(sz)
                .ok_or_else(|| Error::Parse("box size overflow".into()))?;
            if &b[i + 4..i + 8] == want {
                if sz < 8 || box_end > end || box_end > b.len() {
                    return Err(Error::Parse("box size out of range".into()));
                }
                return Ok(Some((i, sz)));
            }
            if sz < 8 {
                break;
            }
            i += sz;
        }
        Ok(None)
    }

    /// Parse a media fragment (moof+mdat bytes) into per-sample decrypt jobs.
    /// Every offset is bounds-checked: a malformed fragment returns an error,
    /// never panics.
    pub fn samples(frag: &[u8]) -> Res<Vec<Sample>> {
        let (moof_i, moof_sz) = find_box(frag, (0, frag.len()), b"moof")?
            .ok_or_else(|| Error::Parse("no moof".into()))?;
        let moof = &frag[moof_i..moof_i + moof_sz];
        let (traf_i, traf_sz) = find_box(moof, (8, moof.len()), b"traf")?
            .ok_or_else(|| Error::Parse("no traf".into()))?;
        let traf = &moof[traf_i..traf_i + traf_sz];

        let mut trun = None;
        let mut senc = None;
        let mut i = 8;
        while i + 8 <= traf.len() {
            let sz = u32b(traf, i)? as usize;
            if sz < 8 || i + sz > traf.len() {
                break; // malformed tail box: stop before slicing out of range
            }
            match &traf[i + 4..i + 8] {
                b"trun" => trun = Some(&traf[i + 8..i + sz]),
                b"senc" => senc = Some(&traf[i + 8..i + sz]),
                _ => {}
            }
            i += sz;
        }
        let trun = trun.ok_or_else(|| Error::Parse("no trun".into()))?;
        let senc = senc.ok_or_else(|| Error::Parse("no senc".into()))?;
        if trun.len() < 8 || senc.len() < 8 {
            return Err(Error::Parse("short trun/senc box".into()));
        }

        // trun
        let fl = u24(&trun[1..4]);
        let count = u32b(trun, 4)? as usize;
        let mut o = 8;
        let data_off = if fl & 1 != 0 {
            let v = u32b(trun, o)? as i32;
            o += 4;
            v
        } else {
            0
        };
        if data_off < 0 {
            return Err(Error::Parse("negative trun data_offset".into()));
        }
        if fl & 4 != 0 {
            o += 4;
        }
        let mut sizes = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            if fl & 0x100 != 0 {
                o += 4;
            }
            if fl & 0x200 != 0 {
                sizes.push(u32b(trun, o)?);
                o += 4;
            }
            if fl & 0x400 != 0 {
                o += 4;
            }
            if fl & 0x800 != 0 {
                o += 4;
            }
        }
        if sizes.len() != count {
            return Err(Error::Parse("trun lacks per-sample sizes".into()));
        }

        // senc: iv_size from tenc is 8 for Apple streams
        let sfl = u24(&senc[1..4]);
        let subsampled = sfl & 2 != 0;
        let scount = u32b(senc, 4)? as usize;
        let mut o = 8;
        let mut out = Vec::with_capacity(scount.min(sizes.len()));
        let mut pos = data_off as u64;
        for &size in sizes.iter() {
            let iv = senc
                .get(o..o + 8)
                .ok_or_else(|| Error::Parse("senc iv out of range".into()))?
                .to_vec();
            o += 8;
            let mut subs = Vec::new();
            if subsampled {
                let n = u16b(senc, o)? as usize;
                o += 2;
                for _ in 0..n {
                    let cb = u16b(senc, o)? as u32;
                    let sb = u32b(senc, o + 2)?;
                    o += 6;
                    subs.push((cb, sb));
                }
            }
            if subs.is_empty() {
                subs.push((0, size));
            }
            out.push(Sample {
                iv,
                subs,
                data_off: pos,
                size,
            });
            pos += size as u64;
        }
        Ok(out)
    }

    /// 24-bit big-endian flags field (trun/senc version+flags bytes 1..4).
    /// Caller guarantees `b.len() >= 3`.
    fn u24(b: &[u8]) -> u32 {
        ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a moof box containing traf(trun+senc) with the given sizes —
    /// lets tests produce structurally valid and malformed fragments.
    fn bx(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
        v.extend_from_slice(name);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn playlist_byterange_without_offset_continues() {
        let pl = "#EXTM3U\n\
            #EXT-X-MAP:URI=\"a.m4a\",BYTERANGE=\"100@0\"\n\
            #EXT-X-BYTERANGE:500@100\n\
            #EXT-X-BYTERANGE:600\n\
            #EXT-X-BYTERANGE:50\n";
        let p = parse_playlist(pl).unwrap();
        assert_eq!(p.segments.len(), 3);
        assert_eq!((p.segments[0].offset, p.segments[0].size), (100, 500));
        assert_eq!((p.segments[1].offset, p.segments[1].size), (600, 600));
        assert_eq!((p.segments[2].offset, p.segments[2].size), (1200, 50));
    }

    #[test]
    fn fmp4_samples_rejects_malformed_boxes() {
        // empty / no moof
        assert!(fmp4::samples(&[]).is_err());
        assert!(fmp4::samples(&bx(b"mdat", &[0u8; 64])).is_err());
        // moof whose declared size overruns the fragment
        let mut bad = Vec::new();
        bad.extend_from_slice(&u32::MAX.to_be_bytes());
        bad.extend_from_slice(b"moof");
        bad.extend_from_slice(&[0u8; 16]);
        assert!(fmp4::samples(&bad).is_err());
        // moof containing a traf whose trun says 4 samples but carries no
        // per-sample size entries — parser must error, not panic
        let mut trun_body = vec![0x00, 0x00, 0x02, 0x01]; // flags 0x201
        trun_body.extend_from_slice(&4u32.to_be_bytes()); // count = 4
        trun_body.extend_from_slice(&128i32.to_be_bytes()); // data_offset
        let traf = {
            let mut t = bx(b"trun", &trun_body);
            t.extend_from_slice(&bx(b"senc", &[0u8; 64]));
            t
        };
        let moof = bx(b"moof", &{
            let mut h = bx(b"mfhd", &[0u8; 8]);
            h.extend_from_slice(&bx(b"traf", &traf));
            h
        });
        let mut frag = moof;
        frag.extend_from_slice(&bx(b"mdat", &[0u8; 256]));
        assert!(fmp4::samples(&frag).is_err());
    }

    #[test]
    fn fmp4_samples_parses_minimal_fragment() {
        // trun: flags 0x201 (data-offset + sample-size), 2 samples of 16B
        let mut trun_body = vec![0x00, 0x00, 0x02, 0x01];
        trun_body.extend_from_slice(&2u32.to_be_bytes());
        trun_body.extend_from_slice(&64i32.to_be_bytes()); // data_offset
        trun_body.extend_from_slice(&16u32.to_be_bytes());
        trun_body.extend_from_slice(&16u32.to_be_bytes());
        // senc: flags=0 (no subsamples), 2 ivs
        let mut senc_body = vec![0x00, 0x00, 0x00, 0x00];
        senc_body.extend_from_slice(&2u32.to_be_bytes());
        senc_body.extend_from_slice(&[0xAA; 8]);
        senc_body.extend_from_slice(&[0xBB; 8]);
        let traf = {
            let mut t = bx(b"trun", &trun_body);
            t.extend_from_slice(&bx(b"senc", &senc_body));
            t
        };
        let mut moof_inner = bx(b"mfhd", &[0u8; 8]);
        moof_inner.extend_from_slice(&bx(b"traf", &traf));
        let mut frag = bx(b"moof", &moof_inner);
        frag.extend_from_slice(&bx(b"mdat", &[0u8; 32]));
        let s = fmp4::samples(&frag).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].iv, vec![0xAA; 8]);
        assert_eq!(s[0].data_off, 64);
        assert_eq!(s[1].data_off, 80);
        assert_eq!(s[0].subs, vec![(0, 16)]);
    }
}

#[cfg(test)]
mod cdm_path_tests {
    use super::validate_cdm_path;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "melba-cdm-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &std::path::Path, name: &str, mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"not-a-real-cdm").unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[test]
    fn accepts_user_owned_regular_so() {
        let dir = scratch();
        let p = write(&dir, "libwidevinecdm.so", 0o644);
        assert!(validate_cdm_path(&p).is_ok(), "{p:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_relative_wrong_name_dir_and_writable() {
        let dir = scratch();
        assert!(validate_cdm_path(std::path::Path::new("libwidevinecdm.so")).is_err());
        let wrong = write(&dir, "libwidevinecdm.so.1", 0o644);
        assert!(validate_cdm_path(&wrong).is_err());
        let other = write(&dir, "other.so", 0o644);
        assert!(validate_cdm_path(&other).is_err());
        let world = write(&dir, "libwidevinecdm.so", 0o666);
        assert!(validate_cdm_path(&world).is_err());
        let group = write(&dir, "libwidevinecdm.so", 0o664);
        assert!(validate_cdm_path(&group).is_err());
        let as_dir = dir.join("nested");
        std::fs::create_dir_all(as_dir.join("libwidevinecdm.so")).unwrap();
        assert!(validate_cdm_path(&as_dir.join("libwidevinecdm.so")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepts_a_securely_installed_system_file() {
        use std::os::unix::fs::MetadataExt;
        let system_file = std::path::Path::new("/bin/sh");
        assert_eq!(std::fs::metadata(system_file).unwrap().uid(), 0);
        let dir = scratch();
        let link = dir.join("libwidevinecdm.so");
        std::os::unix::fs::symlink(system_file, &link).unwrap();
        let result = validate_cdm_path(&link);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn symlink_uses_the_target_owner_and_mode() {
        let dir = scratch();
        let target = write(&dir, "real.so", 0o644);
        let link = dir.join("libwidevinecdm.so");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(validate_cdm_path(&link).is_ok());
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o666);
        std::fs::set_permissions(&target, perms).unwrap();
        assert!(validate_cdm_path(&link).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
