//! Full-track subscription playback: `stream:<id>` queue entries.
//!
//! Resolution order per track:
//!   1. lossless — catalog extendedAssetUrls.enhancedHls → ALAC variant →
//!      cbcs decrypt with content keys from a configured local wrapper
//!      (`~/.config/melba/config.json` → {"wrapper_url": "http://…"}).
//!   2. fallback — webPlayback ctrp (Widevine cenc) AAC-256 → per-sample
//!      decrypt inside the installed browser CDM.
//!
//! Both paths produce one VirtualMedia: a lazily-fetched, decrypted fMP4
//! byte stream handed to symphonia's isomp4 reader — so queueing, seek,
//! gapless stream reuse and the bit-perfect sink pick all behave exactly
//! as for local files.

use std::io::{Read, Seek};
use std::path::PathBuf;
use std::sync::Mutex;

use melba_cdm::stream as ams;
use symphonia::core::io::MediaSource;

use crate::audio::{self, ProbedFile};

/// Parse a `stream:[<storefront>:]<song-id>` queue entry.
pub fn stream_parts(s: &str) -> Option<(String, String)> {
    let rest = s.strip_prefix("stream:")?;
    let mut it = rest.rsplitn(2, ':');
    let id = it.next()?.to_string();
    let cc = it.next().map(String::from);
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let cc = match cc.as_deref() {
        // `stream::id` carries an empty storefront slot — treat it the
        // same as a bare id rather than building a malformed URL
        None | Some("") => crate::musickit::storefront().to_string(),
        Some(c) if c.len() == 2 && c.chars().all(|x| x.is_ascii_alphabetic()) => {
            c.to_ascii_lowercase()
        }
        // anything else in that slot means the URI is malformed, not
        // a stream entry with an exotic storefront
        Some(_) => return None,
    };
    Some((cc, id))
}

/// Locate a Widevine CDM shared object from an installed browser.
pub fn cdm_so() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let mut cands = Vec::new();
    // config.json may override
    if let Some(p) = crate::config::config()["cdm_path"].as_str() {
        cands.push(PathBuf::from(p));
    }
    for base in [
        home.join(".config/vivaldi/WidevineCdm"),
        home.join(".config/google-chrome/WidevineCdm"),
        home.join(".config/chromium/WidevineCdm"),
        home.join(".config/BraveSoftware/Brave-Browser/WidevineCdm"),
    ] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                cands.push(
                    e.path()
                        .join("_platform_specific/linux_x64/libwidevinecdm.so"),
                );
            }
        }
    }
    cands.into_iter().find(|p| p.exists())
}

fn cdm_store() -> Option<PathBuf> {
    crate::config::xdg_dir("XDG_DATA_HOME", ".local/share")
        .map(|p| p.join("melba").join("cdm-store"))
}

/// symphonia MediaSource over the lazy decrypted stream.
pub struct AmSource(Mutex<ams::VirtualMedia>);

impl Read for AmSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // a panicking mid-decode read poisons the lock — recover the
        // guard instead of panicking again, which would kill the
        // decode thread
        let inner = self.0.get_mut().unwrap_or_else(|e| e.into_inner());
        // a panic inside the lazy fetch/decrypt must not unwind through
        // symphonia either — surface it as an io error so the player
        // marks the track failed instead of the thread dying
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.read(buf)))
            .unwrap_or_else(|_| Err(std::io::Error::other("decrypt read panicked")))
    }
}
impl Seek for AmSource {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        let inner = self.0.get_mut().unwrap_or_else(|e| e.into_inner());
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.seek(p)))
            .unwrap_or_else(|_| Err(std::io::Error::other("decrypt seek panicked")))
    }
}
impl MediaSource for AmSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.lock().unwrap_or_else(|e| e.into_inner()).len)
    }
}

/// Stream transport and playlist duration; audio::open_media owns its format.
pub struct Opened {
    source: AmSource,
    dur_ms: u64,
}

impl Opened {
    pub fn probe(self, name: &str) -> Result<ProbedFile, String> {
        let mut pf = audio::open_media(Box::new(self.source), None, name)?;
        // Fragmented MP4 init segments need not declare the full duration.
        pf.src.dur_ms = self.dur_ms;
        Ok(pf)
    }
}

/// Open a `stream:` track. Lossless when a wrapper is configured and the
/// catalog offers ALAC; otherwise AAC-256 via the installed Widevine CDM.
pub fn open(storefront: &str, id: &str) -> Result<Opened, String> {
    match open_once(storefront, id) {
        // a 401 can mean the cached dev token was revoked before its
        // `exp` — invalidate and refetch once before reporting failure.
        Err(e) if e.contains("401") => {
            crate::musickit::invalidate_dev_token();
            open_once(storefront, id)
        }
        r => r,
    }
}

fn open_once(storefront: &str, id: &str) -> Result<Opened, String> {
    let dev = crate::musickit::dev_token()?;
    let user =
        crate::auth::get().ok_or_else(|| "not signed in — run `melbad auth open`".to_string())?;
    let http = ams::Http {
        dev_token: dev,
        user_token: user,
        storefront: storefront.to_string(),
    };
    let wurl = if crate::wrapper::managed() {
        match crate::wrapper::try_ready() {
            Ok(u) => Some(u),
            // a starting/missing service must not block the playback
            // thread (first boot can take minutes) — AAC-256 still
            // plays this subscription track and the next open finds
            // the port listening
            Err(e) => {
                eprintln!("alac unavailable ({e}); falling back to aac-256");
                None
            }
        }
    } else {
        match crate::config::wrapper_url() {
            Some(u) => Some(u),
            None => {
                if let Some(e) = crate::config::wrapper_url_rejection() {
                    eprintln!("alac unavailable ({e}); falling back to aac-256");
                }
                // A staged managed service is usable without a config key:
                // the bootstrap writes the expected layout, so detect it and
                // let try_ready start it. External wrapper_url still wins.
                if crate::wrapper::installed().is_some() {
                    match crate::wrapper::try_ready() {
                        Ok(u) => Some(u),
                        Err(e) => {
                            eprintln!("alac unavailable ({e}); falling back to aac-256");
                            None
                        }
                    }
                } else {
                    None
                }
            }
        }
    };
    if let Some(wurl) = wurl {
        if let Err(e) = melba_cdm::stream::vetted_wrapper_url(&wurl) {
            eprintln!("alac unavailable ({e}); falling back to aac-256");
        } else {
            let h = http.clone();
            let idc = id.to_string();
            let r = std::panic::catch_unwind(move || ams::open_alac(h, &idc, &wurl));
            match r {
                Ok(Ok(vm)) => return Ok(opened(vm)),
                Ok(Err(e)) => eprintln!("alac unavailable ({e}); falling back to aac-256"),
                Err(_) => eprintln!("alac open panicked; falling back to aac-256"),
            }
        }
    }
    let so = cdm_so()
        .ok_or_else(|| "no Widevine CDM found (install Chrome/Chromium/Vivaldi)".to_string())?;
    let store = cdm_store().ok_or_else(|| "no XDG_DATA_HOME or HOME".to_string())?;
    std::fs::create_dir_all(&store).ok();
    let vm = std::panic::catch_unwind(|| ams::open_aac(http, id, &so, &store))
        .map_err(|_| "aac open panicked".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(opened(vm))
}

fn opened(vm: ams::VirtualMedia) -> Opened {
    let dur_ms = vm.dur_ms();
    Opened {
        source: AmSource(Mutex::new(vm)),
        dur_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_parts_handles_edge_uris() {
        assert_eq!(stream_parts("file://x"), None);
        assert_eq!(
            stream_parts("stream:12345"),
            Some((crate::musickit::storefront().to_string(), "12345".into()))
        );
        assert_eq!(
            stream_parts("stream:US:12345"),
            Some(("us".into(), "12345".into()))
        );
        // empty storefront slot degrades to the app's own storefront
        assert_eq!(
            stream_parts("stream::12345"),
            Some((crate::musickit::storefront().to_string(), "12345".into()))
        );
        // non-alpha-2 storefront slot means the URI is malformed
        assert_eq!(stream_parts("stream:a:b:12345"), None);
        // non-numeric ids are not stream entries
        assert_eq!(stream_parts("stream:us:pl.u-abc"), None);
        assert_eq!(stream_parts("stream:"), None);
    }
}
