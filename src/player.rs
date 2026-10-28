//! Slice 03: transport contract — one owner for "control playback from Linux".
//! `melbad serve` = MPRIS daemon on org.mpris.MediaPlayer2.melba; `melbad ctl <verb>`
//! = CLI transport (no daemon needed for play; dbus verbs for control).

use crate::audio::{self, Granted, OpenError, SourceFormat};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

mod order;
use order::PlayOrder;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayState {
    Stopped = 0,
    Playing = 1,
    Paused = 2,
}

/// One playback choice: replace the context without reordering its selected row.
#[derive(Debug)]
pub struct QueueLoad {
    paths: Vec<PathBuf>,
    index: usize,
    position_ms: u32,
    play: bool,
}

#[derive(Debug)]
pub enum Cmd {
    Play,
    Pause,
    Resume,
    Stop,
    Next,
    Prev,
    Quit,
    Seek(u64),
    Shuffle(bool),
    SetLoop(u8), // 0=None 1=Playlist 2=Track
    Enqueue(Vec<PathBuf>),
    LoadTrack(usize),
    /// Move queue entry `from` to final index `to` (caller pre-adjusts for
    /// the removal shift). Keeps the playing entry pinned to `idx`.
    Reorder {
        from: usize,
        to: usize,
    },
    /// Drop every entry except the currently playing one, if any.
    Clear,
    OpenUri(PathBuf), // enqueue + play now
    LoadQueue(QueueLoad),
    /// Retarget the output device at runtime: the current track restarts on
    /// the newly granted sink. `auto` re-enters auto-selection.
    SetDevice(String),
}

/// Structured per-track metadata for MPRIS `xesam:*` fields, keyed by the
/// original queue entry string (same keying as `titles`/`art_urls`).
#[derive(Default, Clone)]
pub struct TrackMeta {
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
}

pub struct Shared {
    pub state: AtomicU8,
    pub pos_ms: std::sync::atomic::AtomicU64,
    pub shuffle: std::sync::atomic::AtomicBool,
    pub repeat: AtomicU8, // 0 none 1 playlist 2 track
    pub dur_ms: std::sync::atomic::AtomicU64,
    /// monotonically increases each time a track starts playing; backs
    /// per-track unique `mpris:trackid` so controllers detect track changes.
    pub play_seq: std::sync::atomic::AtomicU64,
    /// set when Quit is requested; the MPRIS signal watcher exits on it.
    pub quit: std::sync::atomic::AtomicBool,
    /// linear volume 0.0..=1.0 stored as f64 bits; applied by the playback
    /// thread via `audio::apply_gain` (>=0.999 is an exact passthrough).
    pub volume: std::sync::atomic::AtomicU64,
    /// how many ALSA streams this daemon has opened; stays at 1 across
    /// same-format track changes (gapless), grows on format changes.
    pub stream_opens: std::sync::atomic::AtomicU64,
    /// ALSA XRuns recovered while writing. Each one is an audible gap, so it
    /// is counted and published rather than recovered silently.
    pub xruns: std::sync::atomic::AtomicU64,
    /// requested output device ("auto" or hw:X,Y); `melbad ctl device` retargets
    /// it live — the playback thread reads it each open.
    pub device: Mutex<String>,
    pub pending_idx: Mutex<Option<usize>>,
    start_paused: std::sync::atomic::AtomicBool,
    /// (queue idx, ms) seek queued while Stopped — applied when that track
    /// starts; a seek aimed at another idx is dropped.
    pub pending_seek: Mutex<Option<(usize, u64)>>,
    pub queue: Mutex<Vec<PathBuf>>,
    pub titles: Mutex<std::collections::HashMap<String, String>>,
    pub art_urls: Mutex<std::collections::HashMap<String, String>>,
    pub meta: Mutex<std::collections::HashMap<String, TrackMeta>>,
    pub idx: Mutex<usize>,
    order: Mutex<PlayOrder>,
    pub title: Mutex<String>,
    pub art_url: Mutex<String>,
    pub verdict: Mutex<String>,
    pub sink: Mutex<String>,
    /// Refuse shared/default output and any non-bit-perfect verdict.
    pub require_bitperfect: std::sync::atomic::AtomicBool,
    /// Waiting for a device/load command after a bit-perfect refusal.
    pub bitperfect_blocked: std::sync::atomic::AtomicBool,
    /// Format of the source actually opened by the decoder, not the requested
    /// rendition. Published as MPRIS codec/rate/depth for the playing track.
    pub source: Mutex<Option<SourceFormat>>,
}

impl Shared {
    /// Count an ALSA recovery and say so in the journal the first time it
    /// happens, so a gap this process caused is attributable.
    fn note_xrun(&self) {
        let n = self.xruns.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 {
            eprintln!(
                "[player] audio underrun recovered (xruns=1) — the output path did not keep up"
            );
        }
    }

    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(0),
            pos_ms: std::sync::atomic::AtomicU64::new(0),
            shuffle: std::sync::atomic::AtomicBool::new(false),
            repeat: AtomicU8::new(0),
            dur_ms: std::sync::atomic::AtomicU64::new(0),
            play_seq: std::sync::atomic::AtomicU64::new(0),
            quit: std::sync::atomic::AtomicBool::new(false),
            volume: std::sync::atomic::AtomicU64::new(1.0f64.to_bits()),
            stream_opens: std::sync::atomic::AtomicU64::new(0),
            xruns: std::sync::atomic::AtomicU64::new(0),
            device: Mutex::new("auto".into()),
            pending_idx: Mutex::new(None),
            start_paused: std::sync::atomic::AtomicBool::new(false),
            pending_seek: Mutex::new(None),
            queue: Mutex::new(Vec::new()),
            titles: Mutex::new(std::collections::HashMap::new()),
            art_urls: Mutex::new(std::collections::HashMap::new()),
            meta: Mutex::new(std::collections::HashMap::new()),
            idx: Mutex::new(0),
            order: Mutex::new(PlayOrder::default()),
            title: Mutex::new(String::new()),
            art_url: Mutex::new(String::new()),
            verdict: Mutex::new("—".into()),
            sink: Mutex::new("auto".into()),
            require_bitperfect: std::sync::atomic::AtomicBool::new(false),
            bitperfect_blocked: std::sync::atomic::AtomicBool::new(false),
            source: Mutex::new(None),
        })
    }
    pub fn play_state(&self) -> PlayState {
        match self.state.load(Ordering::SeqCst) {
            1 => PlayState::Playing,
            2 => PlayState::Paused,
            _ => PlayState::Stopped,
        }
    }
    fn set_state(&self, s: PlayState) {
        self.state.store(s as u8, Ordering::SeqCst);
    }
}

/// Decoder loop honoring Pause (pcm.pause) / Stop / Next; returns when queue
fn require_block(shared: &Shared) -> bool {
    if !shared.require_bitperfect.load(Ordering::SeqCst) {
        return false;
    }
    shared.bitperfect_blocked.store(true, Ordering::SeqCst);
    shared.set_state(PlayState::Stopped);
    true
}

/// Wait while a required bit-perfect path is unavailable. Commands that can
/// change the device or the selected track return true so the caller retries;
/// everything else is applied (or ignored) without falling back to a mixer.
fn wait_bitperfect(shared: &Arc<Shared>, rx: &Receiver<Cmd>) -> bool {
    loop {
        match rx.recv() {
            Ok(Cmd::Quit) | Err(_) => return false,
            Ok(Cmd::SetDevice(d)) => {
                *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = d;
                return true;
            }
            Ok(Cmd::LoadTrack(i)) => {
                if goto_valid(shared, i) {
                    select_index(shared, i);
                    persist_queue(shared);
                    return true;
                }
            }
            Ok(Cmd::LoadQueue(load)) => {
                if apply_queue_load(shared, load) {
                    return true;
                }
            }
            Ok(Cmd::OpenUri(p)) => {
                let index = {
                    let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                    q.push(p);
                    q.len() - 1
                };
                select_index(shared, index);
                persist_queue(shared);
                return true;
            }
            Ok(Cmd::Clear) => {
                clear_queue(shared);
                return true;
            }
            Ok(Cmd::Next) => {
                advance(shared);
                return true;
            }
            Ok(Cmd::Prev) => {
                previous_index(shared);
                persist_queue(shared);
                return true;
            }
            Ok(Cmd::Play) | Ok(Cmd::Resume) => return true,
            Ok(Cmd::Shuffle(on)) => shared.shuffle.store(on, Ordering::SeqCst),
            Ok(Cmd::SetLoop(mode)) => shared.repeat.store(mode, Ordering::SeqCst),
            Ok(Cmd::Reorder { from, to }) => reorder_queue(shared, from, to),
            Ok(Cmd::Enqueue(ps)) => {
                shared
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(ps);
            }
            Ok(Cmd::Stop) => {
                shared.set_state(PlayState::Stopped);
                shared.pos_ms.store(0, Ordering::SeqCst);
            }
            // No stream is open while blocked; seek/pause/volume are not
            // meaningful until a device opens.
            Ok(_) => {}
        }
    }
}

/// empties or Quit arrives. Playback errors land in `shared.verdict`.
pub fn playback_thread(shared: Arc<Shared>, rx: Receiver<Cmd>, dev: String) {
    // The ALSA stream survives across same-format tracks: the next track's
    // samples append to the still-running stream (gapless album playback).
    // It is drained+dropped on format change and at queue end; a cmd-driven
    // abort (Stop/Next/Prev) leaves it in a dropped state that the next reuse
    // recovers with prepare().
    let mut stream: Option<(alsa::pcm::PCM, audio::Granted)> = None;
    // device string the open stream was granted against; SetDevice retargets it
    let mut cur_dev = shared
        .device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if cur_dev == "auto" && dev != "auto" {
        // launch --device wins over the default
        *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = dev.clone();
        cur_dev = dev.clone();
    }
    let _ = dev; // launch value consumed into shared.device above
    'outer: loop {
        if shared.bitperfect_blocked.load(Ordering::SeqCst) {
            if !wait_bitperfect(&shared, &rx) {
                persist_queue(&shared);
                return;
            }
            shared.bitperfect_blocked.store(false, Ordering::SeqCst);
            continue 'outer;
        }
        let (_idx, path) = {
            // lock discipline: never hold one of these locks while locking the
            // other (advance() takes idx→queue; readers take copies in that
            // order too or serialize the guards). NB the queue guard must be
            // dropped BEFORE rx.recv() — holding it across the blocking wait
            // deadlocks every MPRIS metadata read at end-of-queue.
            let i = *shared.idx.lock().unwrap_or_else(|e| e.into_inner());
            let entry = {
                let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                q.get(i).cloned()
            };
            match entry {
                Some(p) => (i, p),
                None => {
                    // A paused skip carries pause only to a valid next entry,
                    // not to a later explicit append after queue exhaustion.
                    shared.start_paused.store(false, Ordering::SeqCst);
                    if let Some((p, _)) = stream.take() {
                        let _ = p.drain();
                    }
                    shared.set_state(PlayState::Stopped);
                    match rx.recv() {
                        Ok(Cmd::Enqueue(ps)) => {
                            // resume at the first appended entry — the
                            // entries that already played stay behind us
                            let n = {
                                let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                                let n = q.len();
                                q.extend(ps);
                                n
                            };
                            select_index(&shared, n);
                            persist_queue(&shared);
                            continue 'outer;
                        }
                        Ok(Cmd::LoadTrack(i)) => {
                            if goto_valid(&shared, i) {
                                select_index(&shared, i);
                                persist_queue(&shared);
                            }
                            continue 'outer;
                        }
                        Ok(Cmd::LoadQueue(load)) => {
                            apply_queue_load(&shared, load);
                            continue 'outer;
                        }
                        Ok(Cmd::Shuffle(on)) => {
                            shared.shuffle.store(on, Ordering::SeqCst);
                            continue 'outer;
                        }
                        Ok(Cmd::SetLoop(m)) => {
                            shared.repeat.store(m, Ordering::SeqCst);
                            continue 'outer;
                        }
                        Ok(Cmd::Reorder { from, to }) => {
                            reorder_queue(&shared, from, to);
                            continue 'outer;
                        }
                        Ok(Cmd::Clear) => {
                            clear_queue(&shared);
                            continue 'outer;
                        }
                        Ok(Cmd::OpenUri(p)) => {
                            let n = {
                                let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                                q.push(p);
                                q.len() - 1
                            };
                            select_index(&shared, n);
                            persist_queue(&shared);
                            continue 'outer;
                        }
                        Ok(Cmd::SetDevice(d)) => {
                            *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = d;
                            continue 'outer;
                        }
                        Ok(Cmd::Quit) | Err(_) => {
                            persist_queue(&shared);
                            return;
                        }
                        Ok(_) => continue 'outer,
                    }
                }
            }
        };
        // preview:[<cc>:]<apple-id> resolves to the cached 30s AAC download;
        // a deep-link's storefront prefix selects the catalog it came from.
        // stream:[<cc>:]<song-id> is a full subscription track, opened below
        // via crate::stream into a lazily-decrypted fMP4 media source.
        let orig = path.to_string_lossy().to_string();
        // A retained source from a prior occurrence is not evidence this one
        // opened, including when both occurrences have the same URI.
        *shared.source.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let stream_id = crate::stream::stream_parts(&orig);
        if let Some((pstore, pid)) = &stream_id {
            if !shared
                .meta
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&orig)
            {
                if let Ok(v) = crate::musickit::lookup_song_in(pstore, pid) {
                    let nm = v["name"].as_str().unwrap_or(pid);
                    let ar = v["artistName"].as_str().unwrap_or("");
                    let label = if ar.is_empty() {
                        nm.to_string()
                    } else {
                        format!("{ar} — {nm}")
                    };
                    shared
                        .titles
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(orig.clone(), label);
                    shared
                        .meta
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(
                            orig.clone(),
                            TrackMeta {
                                artist: if ar.is_empty() {
                                    None
                                } else {
                                    Some(ar.to_string())
                                },
                                album: v["albumName"].as_str().map(|s| s.to_string()),
                                art_url: v["artworkUrl"].as_str().map(|s| s.to_string()),
                            },
                        );
                }
            }
        }
        let path =
            if let Some((pstore, pid)) = path.to_str().and_then(crate::musickit::preview_parts) {
                // enrich name/artist/album/art from catalog once per id (bare
                // `preview:` URIs from `ctl open`/UI arrive without collection
                // metadata; serve may have already seeded `meta`)
                if !shared
                    .meta
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&orig)
                {
                    if let Ok(v) = crate::musickit::lookup_song_in(&pstore, &pid) {
                        let nm = v["name"].as_str().unwrap_or(&pid);
                        let ar = v["artistName"].as_str().unwrap_or("");
                        let label = if ar.is_empty() {
                            nm.to_string()
                        } else {
                            format!("{ar} — {nm}")
                        };
                        shared
                            .titles
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(orig.clone(), label);
                        shared
                            .meta
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(
                                orig.clone(),
                                TrackMeta {
                                    artist: if ar.is_empty() {
                                        None
                                    } else {
                                        Some(ar.to_string())
                                    },
                                    album: v["albumName"].as_str().map(|s| s.to_string()),
                                    art_url: v["artworkUrl"].as_str().map(|s| s.to_string()),
                                },
                            );
                    }
                }
                match crate::musickit::download_preview_in(&pstore, &pid) {
                    Ok(p) => p,
                    Err(e) => {
                        *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                            format!("preview {pid}: {e}");
                        advance(&shared);
                        continue 'outer;
                    }
                }
            } else {
                // local file: embedded tags + cover art once per path
                if !shared
                    .meta
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&orig)
                {
                    if let Some(fm) = crate::library::file_meta(&path) {
                        if !shared
                            .titles
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .contains_key(&orig)
                        {
                            let label = match (&fm.artist, &fm.title) {
                                (Some(a), Some(t)) => format!("{a} — {t}"),
                                (_, Some(t)) => t.clone(),
                                _ => String::new(),
                            };
                            if !label.is_empty() {
                                shared
                                    .titles
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(orig.clone(), label);
                            }
                        }
                        let art = fm
                            .cover
                            .and_then(|(ext, bytes)| cache_cover(&orig, &ext, &bytes));
                        shared
                            .meta
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(
                                orig.clone(),
                                TrackMeta {
                                    artist: fm.artist,
                                    album: fm.album,
                                    art_url: art,
                                },
                            );
                    }
                }
                path
            };
        // resolved art: explicit seed (collection) wins, else per-track meta
        *shared.art_url.lock().unwrap_or_else(|e| e.into_inner()) = shared
            .art_urls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&orig)
            .cloned()
            .or_else(|| {
                shared
                    .meta
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&orig)
                    .and_then(|m| m.art_url.clone())
            })
            .unwrap_or_default();
        *shared.title.lock().unwrap_or_else(|e| e.into_inner()) = shared
            .titles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&orig)
            .cloned()
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "?".into())
            });
        // media source: every path produces one ProbedFile — opened and
        // probed exactly once (local file, preview m4a, or the stream:
        // AmSource). stream:<id> opens the remote track (ALAC via
        // wrapper, else AAC-256 via CDM). Output and metadata both use
        // the codec configuration validated by audio::open_media.
        let (src, pf): (SourceFormat, audio::ProbedFile) = if let Some((cc, id)) = stream_id {
            match crate::stream::open(&cc, &id) {
                Ok(o) => match o.probe(&orig) {
                    Ok(pf) => (pf.src.clone(), pf),
                    Err(e) => {
                        *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                            format!("stream probe: {e}");
                        if require_block(&shared) {
                            continue 'outer;
                        }
                        advance(&shared);
                        continue 'outer;
                    }
                },
                Err(e) => {
                    // an expired token or a missing wrapper must not
                    // silently skip the track — fall back to the public
                    // 30s preview of the same song and report why
                    match crate::musickit::download_preview_in(&cc, &id) {
                        Ok(p) => {
                            *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                                format!("stream {id}: {e} — playing preview");
                            match audio::open_source(p.to_str().unwrap_or("")) {
                                Ok(pf) => (pf.src.clone(), pf),
                                Err(e) => {
                                    *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                                        format!("preview probe: {e}");
                                    if require_block(&shared) {
                                        continue 'outer;
                                    }
                                    advance(&shared);
                                    continue 'outer;
                                }
                            }
                        }
                        Err(e2) => {
                            *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                                format!("stream {id}: {e} (preview: {e2})");
                            if require_block(&shared) {
                                continue 'outer;
                            }
                            advance(&shared);
                            continue 'outer;
                        }
                    }
                }
            }
        } else {
            match audio::open_source(path.to_str().unwrap_or("")) {
                Ok(pf) => (pf.src.clone(), pf),
                Err(e) => {
                    *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                        format!("probe: {e}");
                    if require_block(&shared) {
                        continue 'outer;
                    }
                    advance(&shared);
                    continue 'outer;
                }
            }
        };
        // Reuse the open stream when it still targets the requested device and
        // carries the new source unchanged (audio::stream_matches); otherwise
        // drain the old tail and reopen.
        let want_dev = shared
            .device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if stream
            .as_ref()
            .is_some_and(|(_, g)| audio::stream_matches(g, &src) && want_dev == cur_dev)
        {
            if let Some((p, _)) = &stream {
                if !matches!(
                    p.state(),
                    alsa::pcm::State::Running | alsa::pcm::State::Prepared
                ) {
                    let _ = p.prepare();
                }
            }
        } else {
            if let Some((p, _)) = stream.take() {
                let _ = p.drain();
            }
            let opened = if want_dev == "auto" {
                open_auto_result(&src)
            } else {
                audio::open_exact(&want_dev, &src)
            };
            match opened {
                Ok(ok) => {
                    shared.stream_opens.fetch_add(1, Ordering::SeqCst);
                    cur_dev = want_dev;
                    stream = Some(ok);
                }
                Err(e) => {
                    *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) =
                        format!("open: {:?}", e);
                    if require_block(&shared) {
                        continue 'outer;
                    }
                    advance(&shared);
                    continue 'outer;
                }
            }
        }
        let (pcm, granted) = stream.as_ref().unwrap();
        *shared.sink.lock().unwrap_or_else(|e| e.into_inner()) = granted.device.clone();
        let verdict = audio::verdict_str(&src, granted);
        *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) = verdict.to_string();
        if verdict != "BITPERFECT" && require_block(&shared) {
            if let Some((p, _)) = stream.take() {
                let _ = p.drain();
            }
            continue 'outer;
        }
        let mut playing_source = src.clone();
        playing_source.path = orig.clone();
        *shared.source.lock().unwrap_or_else(|e| e.into_inner()) = Some(playing_source);
        shared.dur_ms.store(src.dur_ms, Ordering::SeqCst);
        shared.pos_ms.store(0, Ordering::SeqCst);
        shared.play_seq.fetch_add(1, Ordering::SeqCst);
        shared.set_state(PlayState::Playing);
        match play_controlled(pcm, granted, &src, pf, &shared, &rx) {
            Flow::Quit => {
                persist_queue(&shared);
                return;
            }
            Flow::Ended => advance(&shared),
            Flow::Reload => {}
            // Cmd::Stop: hold position in the queue (no advance), stay stopped
            // until a command selects something to play. The stream is left
            // dropped; 'outer's reuse path prepares or reopens it.
            Flow::Stopped => {
                shared.set_state(PlayState::Stopped);
                shared.pos_ms.store(0, Ordering::SeqCst);
                'stopped: loop {
                    match rx.recv() {
                        Ok(Cmd::Play) | Ok(Cmd::Resume) => break 'stopped,
                        Ok(Cmd::Next) => {
                            advance(&shared); // persists internally
                            break 'stopped;
                        }
                        Ok(Cmd::Prev) => {
                            previous_index(&shared);
                            persist_queue(&shared);
                            break 'stopped;
                        }
                        Ok(Cmd::LoadTrack(i)) => {
                            if goto_valid(&shared, i) {
                                select_index(&shared, i);
                                persist_queue(&shared);
                                break 'stopped;
                            }
                        }
                        Ok(Cmd::OpenUri(p)) => {
                            let index = {
                                let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                                q.push(p);
                                q.len() - 1
                            };
                            select_index(&shared, index);
                            persist_queue(&shared);
                            break 'stopped;
                        }
                        Ok(Cmd::LoadQueue(load)) => {
                            if apply_queue_load(&shared, load) {
                                break 'stopped;
                            }
                        }
                        // non-play verbs: take effect, keep waiting
                        Ok(Cmd::Enqueue(ps)) => {
                            shared
                                .queue
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .extend(ps);
                            persist_queue(&shared);
                        }
                        Ok(Cmd::Reorder { from, to }) => reorder_queue(&shared, from, to),
                        Ok(Cmd::Clear) => clear_queue(&shared),
                        Ok(Cmd::SetDevice(d)) => {
                            *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = d
                        }
                        // seek while stopped: park it, applied when this
                        // track next starts (or dropped if another does)
                        Ok(Cmd::Seek(ms)) => {
                            let i = *shared.idx.lock().unwrap_or_else(|e| e.into_inner());
                            *shared
                                .pending_seek
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = Some((i, ms));
                            shared.pos_ms.store(ms, Ordering::SeqCst);
                        }
                        Ok(Cmd::Shuffle(on)) => shared.shuffle.store(on, Ordering::SeqCst),
                        Ok(Cmd::SetLoop(m)) => shared.repeat.store(m, Ordering::SeqCst),
                        Ok(Cmd::Quit) | Err(_) => {
                            persist_queue(&shared);
                            return;
                        }
                        Ok(_) => {}
                    }
                }
            }
        }
    }
}

/// Persist an embedded cover image under the cache dir and return a `file://`
/// URL media controllers can load. Stable per-queue-entry name via sha256.
fn cache_cover(orig: &str, ext: &str, bytes: &[u8]) -> Option<String> {
    use sha2::{Digest, Sha256};
    let dir = crate::config::xdg_dir("XDG_CACHE_HOME", ".cache")?
        .join("melba")
        .join("art");
    std::fs::create_dir_all(&dir).ok()?;
    let h = Sha256::digest(orig.as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    let p = dir.join(format!("{hex}.{ext}"));
    if !p.exists() {
        std::fs::write(&p, bytes).ok()?;
    }
    Some(format!("file://{}", p.display()))
}

/// Commit one load on the playback owner, clearing seeks aimed at the old queue.
fn apply_queue_load(shared: &Shared, load: QueueLoad) -> bool {
    if load.index >= load.paths.len() {
        return false;
    }
    {
        let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
        let mut queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        *queue = load.paths;
        *idx = load.index;
        shared
            .order
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
    }
    *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *shared
        .pending_seek
        .lock()
        .unwrap_or_else(|e| e.into_inner()) =
        (load.position_ms > 0).then_some((load.index, u64::from(load.position_ms)));
    shared.start_paused.store(!load.play, Ordering::SeqCst);
    shared.pos_ms.store(0, Ordering::SeqCst);
    shared.dur_ms.store(0, Ordering::SeqCst);
    *shared.source.lock().unwrap_or_else(|e| e.into_inner()) = None;
    persist_queue(shared);
    true
}

/// New playing index after moving the entry at `from` to post-removal
/// index `to` — the playing track stays pinned to its content.
fn reorder_idx(idx: usize, from: usize, to: usize) -> usize {
    if idx == from {
        to
    } else if from < idx && to >= idx {
        idx - 1
    } else if from > idx && to <= idx {
        idx + 1
    } else {
        idx
    }
}

/// Move queue entry `from` to final index `to` (the caller has already
/// adjusted `to` for the removal shift). Pins `idx` to the playing entry
/// when the move crosses it; persists like every queue mutation.
fn reorder_queue(shared: &Shared, from: usize, to: usize) {
    {
        // lock order is idx→queue everywhere (advance(), metadata_map) —
        // queue→idx here would invert it
        let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        if from >= q.len() {
            return;
        }
        let item = q.remove(from);
        let to = to.min(q.len());
        q.insert(to, item);
        *idx = reorder_idx(*idx, from, to);
        shared
            .order
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remap(|index| reorder_idx(index, from, to));
    }
    persist_queue(shared);
}

/// Empty the queue but keep the playing entry, if any — like clearing
/// the manual queue in spotifast: what is playing keeps playing.
fn clear_queue(shared: &Shared) {
    {
        let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        let keep = (*idx < q.len()).then(|| q[*idx].clone());
        q.clear();
        if let Some(k) = keep {
            q.push(k);
        }
        *idx = 0;
        shared
            .order
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
    }
    persist_queue(shared);
}

/// Only an in-range queue index may move the cursor — a stale goto whose
/// target vanished (reorder/clear raced the request) is dropped rather
/// than parking `idx` past the end of the queue.
fn goto_valid(shared: &Shared, i: usize) -> bool {
    i < shared.queue.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// `~/.local/state/melba/queue.json` — the daemon's queue survives restarts.
/// `None` with no state dir (no HOME): persistence is skipped, not written
/// relative to cwd.
pub(crate) fn queue_state_path() -> Option<PathBuf> {
    crate::config::xdg_dir("XDG_STATE_HOME", ".local/state")
        .map(|b| b.join("melba").join("queue.json"))
}

/// Snapshot queue+idx+titles atomically (tmp + rename). Runs on queue
/// mutation, track advance and Quit — never per packet. Guards are taken
/// sequentially and dropped before the next lock (never nested).
fn persist_queue(shared: &Shared) {
    let Some(path) = queue_state_path() else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let doc = serde_json::json!({
        "v": 1,
        "idx": *shared.idx.lock().unwrap_or_else(|e| e.into_inner()),
        "queue": shared.queue.lock().unwrap_or_else(|e| e.into_inner())
            .iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
        "titles": *shared.titles.lock().unwrap_or_else(|e| e.into_inner()),
    });
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&doc).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
        crate::player::note_queue_write(&path, &doc);
    }
}

/// Append who wrote the queue, beside it.
///
/// The saved queue has been replaced by another process's fixtures twice, and
/// each time the writer had to be guessed from a timestamp. One line per
/// mutation (rare, not per packet) is what makes the next occurrence
/// attributable instead of inferred. Best effort: a failure here never
/// affects playback.
fn note_queue_write(path: &std::path::Path, doc: &serde_json::Value) {
    let Some(parent) = path.parent() else { return };
    let record = serde_json::json!({
        "pid": std::process::id(),
        "ppid": unsafe { libc::getppid() },
        "exe": std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()),
        "args": std::env::args().collect::<Vec<_>>(),
        "state_home": std::env::var("XDG_STATE_HOME").ok(),
        "home": std::env::var("HOME").ok(),
        "idx": doc.get("idx"),
        "len": doc.get("queue").and_then(|q| q.as_array()).map(|q| q.len()),
        "first": doc.get("queue").and_then(|q| q.as_array()).and_then(|q| q.first()).cloned(),
    });
    let mut line = serde_json::to_vec(&record).unwrap_or_default();
    line.push(b'\n');
    use std::io::Write as _;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(parent.join("queue-writes.jsonl"))
    {
        let _ = file.write_all(&line);
    }
}

/// Restore a persisted queue (titles included). True when a non-empty queue
/// came back; the caller keeps `idx` clamped to the queue bounds.
pub fn restore_queue(shared: &Shared) -> bool {
    let Some(path) = queue_state_path() else {
        return false;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let Some(q) = v["queue"].as_array() else {
        return false;
    };
    let paths: Vec<PathBuf> = q
        .iter()
        .filter_map(|s| s.as_str().map(PathBuf::from))
        .collect();
    if paths.is_empty() {
        return false;
    }
    if let Some(t) = v["titles"].as_object() {
        let mut tm = shared.titles.lock().unwrap_or_else(|e| e.into_inner());
        for (k, val) in t {
            if let Some(s) = val.as_str() {
                tm.insert(k.clone(), s.to_string());
            }
        }
    }
    let qlen = paths.len();
    *shared.queue.lock().unwrap_or_else(|e| e.into_inner()) = paths;
    *shared.idx.lock().unwrap_or_else(|e| e.into_inner()) =
        (v["idx"].as_u64().unwrap_or(0) as usize).min(qlen.saturating_sub(1));
    shared
        .order
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reset();
    true
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn open_auto_result(src: &SourceFormat) -> Result<(alsa::pcm::PCM, Granted), OpenError> {
    audio::open_auto(src)
}

#[derive(PartialEq)]
enum Flow {
    Quit,
    /// track finished or was command-aborted → advance()
    Ended,
    /// Cmd::Stop: halt without advancing; daemon waits for a command.
    Stopped,
    /// A load already replaced the queue and selected its exact cursor.
    Reload,
}

/// Cursor writers and MPRIS planning share idx → queue → order lock order.
fn select_index(shared: &Shared, index: usize) {
    let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
    let len = shared.queue.lock().unwrap_or_else(|e| e.into_inner()).len();
    shared
        .order
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .jump(*idx, index, len);
    *idx = index;
}

fn previous_index(shared: &Shared) {
    let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
    let len = shared.queue.lock().unwrap_or_else(|e| e.into_inner()).len();
    *idx = shared
        .order
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .previous(
            *idx,
            len,
            shared.shuffle.load(Ordering::SeqCst),
            shared.repeat.load(Ordering::SeqCst),
        );
}

fn planned_indices(shared: &Shared) -> Vec<u32> {
    let idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
    let len = shared.queue.lock().unwrap_or_else(|e| e.into_inner()).len();
    shared
        .order
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .preview(
            *idx,
            len,
            shared.shuffle.load(Ordering::SeqCst),
            shared.repeat.load(Ordering::SeqCst),
        )
        .into_iter()
        .map(|index| index as u32)
        .collect()
}

fn advance(shared: &Arc<Shared>) {
    {
        let mut idx = shared.idx.lock().unwrap_or_else(|e| e.into_inner());
        let (len, opened) = {
            let queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            let source = shared.source.lock().unwrap_or_else(|e| e.into_inner());
            let opened = source
                .as_ref()
                .zip(queue.get(*idx))
                .is_some_and(|(source, path)| {
                    source.path.as_str() == path.to_string_lossy().as_ref()
                });
            (queue.len(), opened)
        };
        let mut order = shared.order.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = shared
            .pending_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            order.jump(*idx, index, len);
            *idx = index;
        } else {
            *idx = order.next(
                *idx,
                len,
                shared.shuffle.load(Ordering::SeqCst),
                shared.repeat.load(Ordering::SeqCst),
                opened,
            );
        }
    }
    persist_queue(shared);
}

/// Packet-granular control: between packets we can Pause (ALSA pause), Stop,
/// or Quit. Returns Quit if Cmd::Quit seen.
/// symphonia coarse seek to `ms` on the live decoder. Shared by the running
/// Seek handler, Apple-style ⏮ (restart when >3s in), and deferred seeks
/// queued while Stopped.
fn seek_to(
    f: &mut dyn symphonia::core::formats::FormatReader,
    dec: &mut dyn symphonia::core::codecs::Decoder,
    track_id: u32,
    sr: u64,
    ms: u64,
    shared: &Shared,
) {
    use symphonia::core::formats::{SeekMode, SeekTo};
    let ts = ms * sr / 1000;
    if f.seek(SeekMode::Coarse, SeekTo::TimeStamp { ts, track_id })
        .is_ok()
    {
        dec.reset();
        shared.pos_ms.store(ms, Ordering::SeqCst);
    }
}

fn play_controlled(
    pcm: &alsa::pcm::PCM,
    g: &Granted,
    src: &SourceFormat,
    mut probed: audio::ProbedFile,
    shared: &Arc<Shared>,
    rx: &Receiver<Cmd>,
) -> Flow {
    use std::sync::mpsc::TryRecvError;
    let track_id = probed.track_id;
    let mut dec = probed.decoder;
    let io_err = if g.format == "S16LE" {
        pcm.io_i16().err()
    } else {
        pcm.io_i32().err()
    };
    if let Some(e) = io_err {
        *shared.verdict.lock().unwrap_or_else(|e| e.into_inner()) = format!("io: {e}");
        return Flow::Ended;
    }
    let sr = src.rate.max(1) as u64;

    // a seek issued while Stopped (slider drag with no stream open) was
    // parked in pending_seek — honor it iff it targeted this track index.
    if let Some((i, ms)) = shared
        .pending_seek
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        if i == *shared.idx.lock().unwrap_or_else(|e| e.into_inner()) {
            seek_to(&mut *probed.format, &mut *dec, track_id, sr, ms, shared);
        }
    }

    let mut pause_on_start = shared.start_paused.swap(false, Ordering::SeqCst);
    loop {
        let command = if pause_on_start {
            pause_on_start = false;
            Ok(Cmd::Pause)
        } else {
            rx.try_recv()
        };
        match command {
            Ok(Cmd::Pause) => {
                let _ = pcm.pause(true);
                shared.set_state(PlayState::Paused);
                loop {
                    match rx.recv() {
                        Ok(Cmd::Resume) | Ok(Cmd::Play) => {
                            let _ = pcm.pause(false);
                            shared.set_state(PlayState::Playing);
                            break;
                        }
                        Ok(Cmd::Stop) => {
                            let _ = pcm.drop();
                            return Flow::Stopped;
                        }
                        Ok(Cmd::Next) => {
                            shared.start_paused.store(true, Ordering::SeqCst);
                            let _ = pcm.drop();
                            return Flow::Ended;
                        }
                        Ok(Cmd::Prev) => {
                            // Apple Music semantics: >3s in rewinds to the
                            // track start, a press near the start goes back
                            if shared.pos_ms.load(Ordering::SeqCst) > 3000 {
                                seek_to(&mut *probed.format, &mut *dec, track_id, sr, 0, shared);
                                continue;
                            }
                            let _ = pcm.drop();
                            shared.start_paused.store(true, Ordering::SeqCst);
                            previous_index(shared);
                            persist_queue(shared);
                            return Flow::Reload;
                        }
                        Ok(Cmd::SetDevice(d)) => {
                            *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = d;
                            // restart the current track on the new sink
                            let i = *shared.idx.lock().unwrap_or_else(|e| e.into_inner());
                            *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = Some(i);
                            let _ = pcm.drop();
                            return Flow::Ended;
                        }
                        Ok(Cmd::Seek(ms)) => {
                            seek_to(&mut *probed.format, &mut *dec, track_id, sr, ms, shared);
                        }
                        Ok(Cmd::Enqueue(ps)) => {
                            shared
                                .queue
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .extend(ps);
                            persist_queue(shared);
                        }
                        Ok(Cmd::Reorder { from, to }) => reorder_queue(shared, from, to),
                        Ok(Cmd::Clear) => clear_queue(shared),
                        Ok(Cmd::OpenUri(p)) => {
                            let n = {
                                let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                                q.push(p);
                                q.len() - 1
                            };
                            *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = Some(n);
                            let _ = pcm.drop();
                            return Flow::Ended;
                        }
                        Ok(Cmd::LoadQueue(load)) => {
                            if apply_queue_load(shared, load) {
                                let _ = pcm.drop();
                                return Flow::Reload;
                            }
                        }
                        Ok(Cmd::LoadTrack(i)) => {
                            if goto_valid(shared, i) {
                                *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(i);
                                let _ = pcm.drop();
                                return Flow::Ended;
                            }
                        }
                        Ok(Cmd::Shuffle(on)) => shared.shuffle.store(on, Ordering::SeqCst),
                        Ok(Cmd::SetLoop(m)) => shared.repeat.store(m, Ordering::SeqCst),
                        Ok(Cmd::Quit) | Err(_) => return Flow::Quit,
                        Ok(_) => {}
                    }
                }
            }
            Ok(Cmd::Stop) => {
                let _ = pcm.drop();
                return Flow::Stopped;
            }
            Ok(Cmd::Next) => {
                let _ = pcm.drop();
                return Flow::Ended;
            }
            Ok(Cmd::Prev) => {
                // same >3s rule while playing: rewind to 0 and keep playing
                if shared.pos_ms.load(Ordering::SeqCst) > 3000 {
                    seek_to(&mut *probed.format, &mut *dec, track_id, sr, 0, shared);
                    continue;
                }
                let _ = pcm.drop();
                previous_index(shared);
                persist_queue(shared);
                return Flow::Reload;
            }
            Ok(Cmd::SetDevice(d)) => {
                *shared.device.lock().unwrap_or_else(|e| e.into_inner()) = d;
                let i = *shared.idx.lock().unwrap_or_else(|e| e.into_inner());
                *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = Some(i);
                let _ = pcm.drop();
                return Flow::Ended;
            }
            Ok(Cmd::Quit) | Err(TryRecvError::Disconnected) => return Flow::Quit,
            Ok(Cmd::Seek(ms)) => {
                seek_to(&mut *probed.format, &mut *dec, track_id, sr, ms, shared);
            }
            Ok(Cmd::Shuffle(on)) => shared.shuffle.store(on, Ordering::SeqCst),
            Ok(Cmd::SetLoop(m)) => shared.repeat.store(m, Ordering::SeqCst),
            Ok(Cmd::Enqueue(ps)) => {
                shared
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(ps);
                persist_queue(shared);
            }
            Ok(Cmd::Reorder { from, to }) => reorder_queue(shared, from, to),
            Ok(Cmd::Clear) => clear_queue(shared),
            Ok(Cmd::OpenUri(p)) => {
                let n = {
                    let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                    q.push(p);
                    q.len() - 1
                };
                *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = Some(n);
                let _ = pcm.drop();
                return Flow::Ended;
            }
            Ok(Cmd::LoadQueue(load)) => {
                if apply_queue_load(shared, load) {
                    let _ = pcm.drop();
                    return Flow::Reload;
                }
            }
            Ok(Cmd::LoadTrack(i)) if goto_valid(shared, i) => {
                *shared.pending_idx.lock().unwrap_or_else(|e| e.into_inner()) = Some(i);
                let _ = pcm.drop();
                return Flow::Ended;
            }
            _ => {}
        }

        let packet = match probed.format.next_packet() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[player] next_packet: {e}");
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[player] decode err: {e}");
                continue;
            }
        };
        let mut samples = audio::decoded_to_i32(decoded);
        audio::apply_gain(
            &mut samples,
            f64::from_bits(shared.volume.load(Ordering::Relaxed)),
        );
        if g.format == "S16LE" {
            let io = match pcm.io_i16() {
                Ok(i) => i,
                Err(_) => return Flow::Ended,
            };
            let out: Vec<i16> = samples.iter().map(|&s| (s >> 16) as i16).collect();
            if write_all_i16(&io, pcm, &out, g.channels as usize, shared).is_err() {
                return Flow::Ended;
            }
        } else {
            let io = match pcm.io_i32() {
                Ok(i) => i,
                Err(_) => return Flow::Ended,
            };
            if write_all_i32(&io, pcm, &samples, g.channels as usize, shared).is_err() {
                return Flow::Ended;
            }
        }
        shared.pos_ms.fetch_add(
            samples.len() as u64 / g.channels.max(1) as u64 * 1000 / sr,
            Ordering::SeqCst,
        );
    }
    // No drain here: the caller keeps the stream open for the next
    // same-format track (gapless); it drains when the stream actually closes.
    Flow::Ended
}

/// Write `samples` in full, recovering an XRun as an audible gap but never
/// hiding one: `xruns` counts every recovery so `melbad now` and the log can
/// show that the output path is not keeping up.
fn write_all_i16(
    io: &alsa::pcm::IO<'_, i16>,
    pcm: &alsa::pcm::PCM,
    samples: &[i16],
    ch: usize,
    shared: &Shared,
) -> Result<(), ()> {
    let mut done = 0usize;
    while done * ch < samples.len() {
        match io.writei(&samples[done * ch..]) {
            // 0 accepted forever would spin with no progress
            Ok(0) => return Err(()),
            Ok(n) => done += n,
            Err(e) => {
                shared.note_xrun();
                if pcm.try_recover(e, true).is_err() {
                    return Err(());
                }
            }
        }
    }
    Ok(())
}
fn write_all_i32(
    io: &alsa::pcm::IO<'_, i32>,
    pcm: &alsa::pcm::PCM,
    samples: &[i32],
    ch: usize,
    shared: &Shared,
) -> Result<(), ()> {
    let mut done = 0usize;
    while done * ch < samples.len() {
        match io.writei(&samples[done * ch..]) {
            Ok(0) => return Err(()),
            Ok(n) => done += n,
            Err(e) => {
                shared.note_xrun();
                if pcm.try_recover(e, true).is_err() {
                    return Err(());
                }
            }
        }
    }
    Ok(())
}

// ---- MPRIS daemon ----------------------------------------------------------

fn player_status(shared: &Shared) -> String {
    match shared.play_state() {
        PlayState::Playing => "Playing".into(),
        PlayState::Paused => "Paused".into(),
        PlayState::Stopped => "Stopped".into(),
    }
}

/// URI a media controller can display for the current queue entry.
fn track_uri(orig: &str) -> String {
    if orig.contains("://") || orig.starts_with("preview:") || orig.starts_with("stream:") {
        orig.to_string()
    } else {
        let mut u = String::from("file://");
        for b in orig.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' | b'~' => {
                    u.push(*b as char)
                }
                _ => u.push_str(&format!("%{b:02X}")),
            }
        }
        u
    }
}

/// Full MPRIS `Metadata` dict for the current track — shared by the property
/// getter and the PropertiesChanged watcher so signals carry identical data.
fn metadata_map(shared: &Shared) -> dbus::arg::PropMap {
    use dbus::arg::{RefArg, Variant};
    let mut m: dbus::arg::PropMap = std::collections::HashMap::new();
    let mut put = |k: &str, v: Box<dyn RefArg>| {
        m.insert(k.to_string(), Variant(v));
    };
    let orig = {
        let i = *shared.idx.lock().unwrap_or_else(|e| e.into_inner());
        let q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.get(i).map(|p| p.to_string_lossy().to_string())
    };
    let seq = shared.play_seq.load(Ordering::SeqCst);
    put(
        "mpris:trackid",
        Box::new(dbus::Path::from(format!(
            "/org/mpris/MediaPlayer2/melba/Track/{seq}"
        ))),
    );
    put(
        "xesam:title",
        Box::new(
            shared
                .title
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        ),
    );
    let tm = orig.as_ref().and_then(|o| {
        shared
            .meta
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(o)
            .cloned()
    });
    put(
        "xesam:artist",
        Box::new(
            tm.as_ref()
                .and_then(|m| m.artist.clone())
                .map(|a| vec![a])
                .unwrap_or_default(),
        ),
    );
    put(
        "xesam:album",
        Box::new(
            tm.as_ref()
                .and_then(|m| m.album.clone())
                .unwrap_or_default(),
        ),
    );
    if let Some(o) = &orig {
        put("xesam:url", Box::new(track_uri(o)));
    }
    put(
        "mpris:length",
        Box::new(shared.dur_ms.load(Ordering::SeqCst) as i64 * 1000),
    );
    put(
        "mpris:artUrl",
        Box::new(
            shared
                .art_url
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        ),
    );
    put(
        "melba:verdict",
        Box::new(
            shared
                .verdict
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        ),
    );
    put(
        "melba:sink",
        Box::new(
            shared
                .sink
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        ),
    );
    if let Some(source) = shared
        .source
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|source| orig.as_deref() == Some(source.path.as_str()))
    {
        put("melba:codec", Box::new(source.codec.clone()));
        put("melba:sampleRate", Box::new(source.rate));
        put("melba:bitDepth", Box::new(source.bits));
        put("melba:channels", Box::new(source.channels));
    }
    put(
        "melba:device",
        Box::new(
            shared
                .device
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        ),
    );
    put(
        "melba:volume",
        Box::new(f64::from_bits(shared.volume.load(Ordering::SeqCst))),
    );
    put(
        "melba:streamOpens",
        Box::new(shared.stream_opens.load(Ordering::SeqCst)),
    );
    put("melba:xruns", Box::new(shared.xruns.load(Ordering::SeqCst)));
    put(
        "melba:queue",
        Box::new(
            shared
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<String>>(),
        ),
    );
    put(
        "melba:idx",
        Box::new(*shared.idx.lock().unwrap_or_else(|e| e.into_inner()) as u32),
    );
    put("melba:nextIndices", Box::new(planned_indices(shared)));
    {
        let paths: Vec<String> = shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let tm = shared.titles.lock().unwrap_or_else(|e| e.into_inner());
        put(
            "melba:titles",
            Box::new(
                paths
                    .iter()
                    .map(|k| {
                        tm.get(k).cloned().unwrap_or_else(|| {
                            PathBuf::from(k)
                                .file_stem()
                                .map(|x| x.to_string_lossy().to_string())
                                .unwrap_or_else(|| k.clone())
                        })
                    })
                    .collect::<Vec<String>>(),
            ),
        );
        // per-entry enrichment the UI would otherwise have to re-derive
        // from catalog lookups — empty strings where not (yet) known.
        let meta = shared.meta.lock().unwrap_or_else(|e| e.into_inner());
        put(
            "melba:artists",
            Box::new(
                paths
                    .iter()
                    .map(|k| {
                        meta.get(k)
                            .and_then(|m| m.artist.clone())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<String>>(),
            ),
        );
        let fallback_art_urls: Vec<_> = paths
            .iter()
            .map(|k| meta.get(k).and_then(|m| m.art_url.clone()))
            .collect();
        // Playback reads art_urls before meta; never wait for art_urls while
        // retaining meta, or the receive loop and decoder can deadlock.
        drop(meta);
        let arts = shared.art_urls.lock().unwrap_or_else(|e| e.into_inner());
        put(
            "melba:artUrls",
            Box::new(
                paths
                    .iter()
                    .zip(fallback_art_urls)
                    .map(|(k, fallback)| arts.get(k).cloned().or(fallback).unwrap_or_default())
                    .collect::<Vec<String>>(),
            ),
        );
    }
    m
}

/// Serve org.mpris.MediaPlayer2.melba on the session bus. Blocking; runs until
/// Quit. `ctl` sends verbs into the playback thread.
pub fn serve(
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    rx: Receiver<Cmd>,
    device: String,
    paused: bool,
    require_bitperfect: bool,
) -> Result<(), String> {
    use dbus::blocking::Connection;
    use dbus::channel::MatchingReceiver;
    use dbus::message::MatchRule;
    use std::time::Duration;

    let conn = Connection::new_session().map_err(|e| format!("dbus: {e}"))?;
    use dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply;
    let got = conn
        .request_name("org.mpris.MediaPlayer2.melba", false, false, false)
        .map_err(|e| format!("request_name: {e}"))?;
    if !matches!(
        got,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    ) {
        return Err(
            "org.mpris.MediaPlayer2.melba is already owned — another `melbad serve` \
             is running (use `melbad ctl quit` to stop it)"
                .into(),
        );
    }
    // Never open audio or restore playback before owning the control name.
    shared
        .require_bitperfect
        .store(require_bitperfect, Ordering::SeqCst);
    shared.start_paused.store(paused, Ordering::SeqCst);
    let playback = shared.clone();
    std::thread::spawn(move || playback_thread(playback, rx, device));
    let tx = Arc::new(Mutex::new(tx));
    let shared2 = shared.clone();

    let mut cr = dbus_crossroads::Crossroads::new();
    let txp_root = tx.clone();
    let iface = cr.register("org.mpris.MediaPlayer2.Player", move |b| {
        let txp = tx.clone();
        b.method("Play", (), (), move |_, _: &mut St, _: ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Play)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        b.method("Pause", (), (), move |_, _: &mut St, _: ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Pause)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        b.method("PlayPause", (), (), move |_, s: &mut St, _: ()| {
            let st = s.shared.play_state();
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(if st == PlayState::Playing {
                    Cmd::Pause
                } else {
                    Cmd::Play
                })
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        // Authentication may release only the service this daemon owns.
        b.method(
            "MelbaStopManagedWrapper",
            (),
            (),
            move |_, _: &mut St, _: ()| {
                crate::wrapper::stop().map_err(|e| dbus::MethodErr::failed(&e))?;
                Ok(())
            },
        );
        b.method("Stop", (), (), move |_, _: &mut St, _: ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Stop)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        b.method("Next", (), (), move |_, _: &mut St, _: ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Next)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        b.method("Previous", (), (), move |_, _: &mut St, _: ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Prev)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        b.method(
            "Seek",
            ("x",),
            (),
            move |ctx: &mut dbus_crossroads::Context, s: &mut St, (x,): (i64,)| {
                let cur_ms = s.shared.pos_ms.load(Ordering::SeqCst) as i64;
                let target = (cur_ms + x / 1000).max(0) as u64;
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::Seek(target))
                    .ok();
                // MPRIS: Seeked carries the new position in microseconds
                if let Ok(sig) = dbus::Message::new_signal(
                    "/org/mpris/MediaPlayer2",
                    "org.mpris.MediaPlayer2.Player",
                    "Seeked",
                ) {
                    ctx.push_msg(sig.append1(target as i64 * 1000));
                }
                Ok(())
            },
        );
        let txp = tx.clone();
        b.method(
            "SetPosition",
            ("o", "x"),
            (),
            move |ctx: &mut dbus_crossroads::Context,
                  s: &mut St,
                  (o, x): (dbus::Path<'static>, i64)| {
                // spec: SetPosition on a stale trackid does nothing —
                // a controller that seeks a track we already left must
                // not move the new one
                let cur = format!(
                    "/org/mpris/MediaPlayer2/melba/Track/{}",
                    s.shared.play_seq.load(Ordering::SeqCst)
                );
                if o.to_string() != cur {
                    return Ok(());
                }
                let target = (x / 1000).max(0) as u64;
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::Seek(target))
                    .ok();
                if let Ok(sig) = dbus::Message::new_signal(
                    "/org/mpris/MediaPlayer2",
                    "org.mpris.MediaPlayer2.Player",
                    "Seeked",
                ) {
                    ctx.push_msg(sig.append1(x.max(0)));
                }
                Ok(())
            },
        );
        let txp = tx.clone();
        b.method(
            "OpenUri",
            ("s",),
            (),
            move |_, _: &mut St, (uri,): (String,)| {
                if let Some(p) = uri.strip_prefix("file://") {
                    let dec = percent_decode(p);
                    txp.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .send(Cmd::OpenUri(dec.into()))
                        .ok();
                } else if uri.starts_with("preview:") || uri.starts_with("stream:") {
                    txp.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .send(Cmd::OpenUri(uri.into()))
                        .ok();
                } else if crate::frontend::is_link(&uri) {
                    crate::frontend::forward(Some(&uri))
                        .map_err(|error| dbus::MethodErr::failed(&error))?;
                } else {
                    return Err(dbus::MethodErr::invalid_arg("unsupported playback URI"));
                }
                Ok(())
            },
        );
        let txp = tx.clone();
        b.method(
            "MelbaLoadQueue",
            ("uris", "index", "position_ms", "play"),
            (),
            move |_,
                  _: &mut St,
                  (uris, index, position_ms, play): (Vec<String>, u32, u32, bool)| {
                if uris.is_empty() || index as usize >= uris.len() {
                    return Err(dbus::MethodErr::invalid_arg("queue selection out of range"));
                }
                let paths: Option<Vec<PathBuf>> = uris
                    .iter()
                    .map(|uri| {
                        if let Some(path) = uri.strip_prefix("file://") {
                            Some(percent_decode(path).into())
                        } else if uri.starts_with("stream:") || uri.starts_with("preview:") {
                            Some(uri.into())
                        } else {
                            None
                        }
                    })
                    .collect();
                let paths =
                    paths.ok_or_else(|| dbus::MethodErr::invalid_arg("unsupported queue URI"))?;
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::LoadQueue(QueueLoad {
                        paths,
                        index: index as usize,
                        position_ms,
                        play,
                    }))
                    .map_err(|_| dbus::MethodErr::failed("playback worker unavailable"))?;
                Ok(())
            },
        );
        let txp = tx.clone();
        b.method(
            "MelbaGoTo",
            ("u",),
            (),
            move |_, _: &mut St, (i,): (u32,)| {
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::LoadTrack(i as usize))
                    .ok();
                Ok(())
            },
        );
        let txp = tx.clone();
        // move entry `from` to final index `to`; the queue persists and the
        // playing entry stays pinned (idx follows it across the move)
        b.method(
            "MelbaQueueReorder",
            ("u", "u"),
            (),
            move |_, _: &mut St, (from, to): (u32, u32)| {
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::Reorder {
                        from: from as usize,
                        to: to as usize,
                    })
                    .ok();
                Ok(())
            },
        );
        let txp = tx.clone();
        b.method("MelbaQueueClear", (), (), move |_, _: &mut St, ()| {
            txp.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(Cmd::Clear)
                .ok();
            Ok(())
        });
        let txp = tx.clone();
        // append without disturbing what is playing — the "add to queue"
        // half of OpenUri (same accepted URI schemes).
        b.method(
            "MelbaEnqueue",
            ("as",),
            (),
            move |_, _: &mut St, (uris,): (Vec<String>,)| {
                let ps: Vec<PathBuf> = uris
                    .iter()
                    .filter_map(|u| {
                        if let Some(p) = u.strip_prefix("file://") {
                            Some(percent_decode(p).into())
                        } else if u.starts_with("preview:") || u.starts_with("stream:") {
                            Some(u.into())
                        } else {
                            None
                        }
                    })
                    .collect();
                if !ps.is_empty() {
                    txp.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .send(Cmd::Enqueue(ps))
                        .ok();
                }
                Ok(())
            },
        );
        // Machine-readable audio outputs for the settings picker. This is
        // an explicit request, not part of the frequently-read metadata.
        b.method("MelbaAudioDevices", (), ("s",), |_, s: &mut St, _: ()| {
            let current = s
                .shared
                .device
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let devices = crate::audio::list_devices();
            let payload = serde_json::json!({ "current": current, "devices": devices });
            Ok((serde_json::to_string(&payload)
                .unwrap_or_else(|_| "{\"current\":\"auto\",\"devices\":[]}".into()),))
        });
        // Persist the picker choice and retarget the live daemon.
        let txp = tx.clone();
        b.method(
            "MelbaSetAudioDevice",
            ("s",),
            (),
            move |_, s: &mut St, (device,): (String,)| {
                let sink = s
                    .shared
                    .sink
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if device != sink && device.starts_with("hw:") {
                    audio::release_for_takeover(&device)
                        .map_err(|error| dbus::MethodErr::failed(&error))?;
                }
                crate::config::set_audio_device(&device)
                    .map_err(|e| dbus::MethodErr::invalid_arg(&e))?;
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::SetDevice(device))
                    .ok();
                Ok(())
            },
        );
        // runtime output-device switch: "hw:X,Y" or "auto"; restarts the
        // current track on the newly granted sink
        let txp = tx.clone();
        b.method(
            "MelbaSetDevice",
            ("s",),
            (),
            move |_, s: &mut St, (d,): (String,)| {
                let sink = s
                    .shared
                    .sink
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if d != sink && d.starts_with("hw:") {
                    audio::release_for_takeover(&d)
                        .map_err(|error| dbus::MethodErr::failed(&error))?;
                }
                txp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::SetDevice(d))
                    .ok();
                Ok(())
            },
        );
        b.property("PlaybackStatus")
            .emits_changed_true()
            .get(|_, s: &mut St| Ok(player_status(&s.shared)));
        // per spec Position changes are reported via the Seeked signal only
        b.property("Position")
            .emits_changed_false()
            .get(|_, s: &mut St| Ok(s.shared.pos_ms.load(Ordering::SeqCst) as i64 * 1000));
        b.property("Metadata")
            .emits_changed_true()
            .get(|_, s: &mut St| Ok(metadata_map(&s.shared)));
        b.property("LoopStatus")
            .emits_changed_true()
            .get(|_, s: &mut St| {
                Ok(match s.shared.repeat.load(Ordering::SeqCst) {
                    1 => "Playlist".to_string(),
                    2 => "Track".to_string(),
                    _ => "None".to_string(),
                })
            })
            .set(|_, s: &mut St, v: String| {
                let m = match v.as_str() {
                    "Playlist" => 1u8,
                    "Track" => 2u8,
                    _ => 0u8,
                };
                s.shared.repeat.store(m, Ordering::SeqCst);
                Ok(Some(v))
            });
        b.property("Shuffle")
            .emits_changed_true()
            .get(|_, s: &mut St| Ok(s.shared.shuffle.load(Ordering::SeqCst)))
            .set(|_, s: &mut St, v: bool| {
                s.shared.shuffle.store(v, Ordering::SeqCst);
                Ok(Some(v))
            });
        b.property("Rate")
            .emits_changed_false()
            .get(|_, _: &mut St| Ok(1.0f64))
            .set(|_, _: &mut St, _| Ok(None));
        b.property("MinimumRate").get(|_, _: &mut St| Ok(1.0f64));
        b.property("MaximumRate").get(|_, _: &mut St| Ok(1.0f64));
        b.property("CanSeek").get(|_, _: &mut St| Ok(true));
        b.property("CanPlay").get(|_, _: &mut St| Ok(true));
        b.property("CanPause").get(|_, _: &mut St| Ok(true));
        // Capabilities and transport consult the same navigation plan.
        b.property("CanGoNext")
            .get(|_, s: &mut St| Ok(!planned_indices(&s.shared).is_empty()));
        b.property("CanGoPrevious").get(|_, s: &mut St| {
            Ok(!s
                .shared
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty())
        });
        b.property("CanControl").get(|_, _: &mut St| Ok(true));
        b.property("Volume")
            .emits_changed_true()
            .get(|_, s: &mut St| Ok(f64::from_bits(s.shared.volume.load(Ordering::SeqCst))))
            .set(|_, s: &mut St, v: f64| {
                let v = v.clamp(0.0, 1.0);
                s.shared.volume.store(v.to_bits(), Ordering::SeqCst);
                // Some(v) lets crossroads emit PropertiesChanged —
                // returning None would silently swallow the update for
                // every external volume display
                Ok(Some(v))
            });
    });
    let root_iface = {
        let txq = txp_root;
        cr.register("org.mpris.MediaPlayer2", move |b| {
            let txq2 = txq.clone();
            b.method("Quit", (), (), move |_, s: &mut St, _: ()| {
                s.shared.quit.store(true, Ordering::SeqCst);
                txq2.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(Cmd::Quit)
                    .ok();
                Ok(())
            });
            b.method("Raise", (), (), |_, _: &mut St, _: ()| {
                crate::frontend::forward(None).map_err(|error| dbus::MethodErr::failed(&error))
            });
            b.property("CanQuit").get(|_, _: &mut St| Ok(true));
            b.property("CanRaise").get(|_, _: &mut St| Ok(true));
            b.property("HasTrackList").get(|_, _: &mut St| Ok(false));
            b.property("Identity")
                .get(|_, _: &mut St| Ok("Melba".to_string()));
            b.property("DesktopEntry")
                // basename of packaging/melba.desktop — Plasma's media
                // applet resolves icon/name through it
                .get(|_, _: &mut St| Ok("melba".to_string()));
            b.property("SupportedUriSchemes").get(|_, _: &mut St| {
                Ok(vec![
                    "file".to_string(),
                    "preview".to_string(),
                    "stream".to_string(),
                    "melba".to_string(),
                    "spotify".to_string(),
                    "http".to_string(),
                    "https".to_string(),
                    "music".to_string(),
                    "musics".to_string(),
                    "apple-music".to_string(),
                    "itms".to_string(),
                    "itmss".to_string(),
                ])
            });
            b.property("SupportedMimeTypes").get(|_, _: &mut St| {
                Ok(vec![
                    "audio/flac".to_string(),
                    "audio/x-m4a".to_string(),
                    "audio/mpeg".to_string(),
                    "audio/ogg".to_string(),
                    "audio/wav".to_string(),
                    "audio/x-caf".to_string(),
                ])
            });
        })
    };
    struct St {
        shared: Arc<Shared>,
    }
    let state = St { shared: shared2 };
    cr.insert("/org/mpris/MediaPlayer2", &[iface, root_iface], state);

    conn.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |msg, conn| {
            let _ = cr.handle_message(msg, conn);
            true
        }),
    );

    // PropertiesChanged publishing: track/state changes driven by the playback
    // thread (auto-advance, ctl verbs) never pass through a method handler, so
    // the receive loop polls them and emits the signal itself — media widgets
    // depend on these to notice track changes without polling.
    let mut last_sig_state: Option<(u8, u64, u64)> = None;
    loop {
        if conn.process(Duration::from_millis(400)).is_err() {
            break;
        }
        if shared.quit.load(Ordering::SeqCst) {
            break;
        }
        // verdict/device/sink mutate on the playback thread without
        // bumping play_seq — fold them into the watch tuple so e.g. a
        // BITPERFECT verdict reaching the controller isn't silent
        let cur = (
            shared.state.load(Ordering::SeqCst),
            shared.play_seq.load(Ordering::SeqCst),
            {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                shared
                    .verdict
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hash(&mut h);
                shared
                    .device
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hash(&mut h);
                shared
                    .sink
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hash(&mut h);
                h.finish()
            },
        );
        if last_sig_state != Some(cur) {
            last_sig_state = Some(cur);
            let mut changed: dbus::arg::PropMap = std::collections::HashMap::new();
            changed.insert(
                "PlaybackStatus".into(),
                dbus::arg::Variant(Box::new(player_status(&shared))),
            );
            changed.insert(
                "Metadata".into(),
                dbus::arg::Variant(Box::new(metadata_map(&shared))),
            );
            if let Ok(sig) = dbus::Message::new_signal(
                "/org/mpris/MediaPlayer2",
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
            ) {
                let _ = conn.channel().send(sig.append3(
                    "org.mpris.MediaPlayer2.Player".to_string(),
                    changed,
                    Vec::<String>::new(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reorder_idx;

    /// The recorder is the evidence a future occurrence is judged on, so its
    /// shape is pinned: without the process identity it cannot name a writer.
    #[test]
    fn a_queue_write_records_who_wrote_it() {
        let dir = std::env::temp_dir().join(format!("melba-queue-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queue.json");
        let doc = serde_json::json!({"idx": 2, "queue": ["stream:m0", "stream:new"], "titles": {}});
        crate::player::note_queue_write(&path, &doc);
        let text = std::fs::read_to_string(dir.join("queue-writes.jsonl")).unwrap();
        let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(line["pid"].as_u64().unwrap(), std::process::id() as u64);
        assert!(line["ppid"].as_u64().is_some(), "{line}");
        assert!(
            line["exe"].as_str().is_some_and(|e| !e.is_empty()),
            "{line}"
        );
        assert!(
            line["args"].as_array().is_some_and(|a| !a.is_empty()),
            "{line}"
        );
        assert_eq!(line["len"], 2);
        assert_eq!(line["first"], "stream:m0");
        assert_eq!(line["idx"], 2);
        // Appending, not overwriting: a second write keeps the first.
        crate::player::note_queue_write(&path, &doc);
        assert_eq!(
            std::fs::read_to_string(dir.join("queue-writes.jsonl"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_reports_the_opened_source_not_the_requested_quality() {
        let shared = super::Shared::new();
        shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("stream:123".into());
        assert!(!super::metadata_map(&shared).contains_key("melba:codec"));
        for (codec, rate, bits) in [("alac", 96000, 24), ("mp4a", 44100, 16)] {
            *shared.source.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(crate::audio::SourceFormat {
                    path: "stream:123".into(),
                    codec: codec.into(),
                    rate,
                    bits,
                    channels: 2,
                    dur_ms: 1000,
                });
            let meta = super::metadata_map(&shared);
            assert_eq!(meta["melba:codec"].0.as_str(), Some(codec));
            assert_eq!(meta["melba:sampleRate"].0.as_u64(), Some(u64::from(rate)));
            assert_eq!(meta["melba:bitDepth"].0.as_u64(), Some(u64::from(bits)));
            assert_eq!(meta["melba:channels"].0.as_u64(), Some(2));
        }
        shared.queue.lock().unwrap_or_else(|e| e.into_inner())[0] = "stream:456".into();
        assert!(
            !super::metadata_map(&shared).contains_key("melba:codec"),
            "the next track must not inherit the previous stream's quality while opening"
        );
    }

    #[test]
    fn metadata_does_not_hold_track_metadata_while_waiting_for_artwork() {
        use std::sync::{mpsc, TryLockError};
        use std::time::{Duration, Instant};
        let shared = super::Shared::new();
        shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("fixture".into());
        shared
            .meta
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                "fixture".into(),
                super::TrackMeta {
                    artist: Some("Fixture artist".into()),
                    art_url: Some("fixture:art".into()),
                    ..Default::default()
                },
            );
        let artwork = shared.art_urls.lock().unwrap_or_else(|e| e.into_inner());
        let (ready, started) = mpsc::channel();
        let reader = shared.clone();
        let worker = std::thread::spawn(move || {
            let tid = unsafe { libc::syscall(libc::SYS_gettid) };
            ready.send(tid).unwrap();
            super::metadata_map(&reader)
        });
        let tid = started.recv_timeout(Duration::from_secs(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut waiting_for_artwork = false;
        while Instant::now() < deadline {
            let publishing = matches!(shared.titles.try_lock(), Err(TryLockError::WouldBlock));
            let wait =
                std::fs::read_to_string(format!("/proc/self/task/{tid}/wchan")).unwrap_or_default();
            if publishing && wait.contains("futex") {
                waiting_for_artwork = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // Playback looks up art_urls before meta. The reverse overlap in this
        // reader would deadlock both threads; inspect the real blocked reader.
        let metadata_available = shared.meta.try_lock().is_ok();
        drop(artwork);
        let result = worker.join().unwrap();
        assert!(
            waiting_for_artwork,
            "metadata reader never reached the controlled artwork wait"
        );
        assert!(
            metadata_available,
            "metadata remained locked while the reader waited for artwork"
        );
        let urls: Vec<_> = result["melba:artUrls"]
            .0
            .as_iter()
            .unwrap()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(urls, ["fixture:art"]);
    }

    #[test]
    fn bitperfect_block_waits_for_a_device_or_load_and_honours_quit() {
        use std::sync::mpsc;
        let shared = super::Shared::new();
        assert!(!super::require_block(&shared));
        shared
            .require_bitperfect
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(super::require_block(&shared));
        assert!(shared
            .bitperfect_blocked
            .load(std::sync::atomic::Ordering::SeqCst));
        let (tx, rx) = mpsc::channel();
        tx.send(super::Cmd::SetDevice("hw:3,3".into())).unwrap();
        assert!(super::wait_bitperfect(&shared, &rx));
        assert_eq!(
            *shared.device.lock().unwrap_or_else(|e| e.into_inner()),
            "hw:3,3",
            "a device command must retry the same track on the new sink"
        );
        tx.send(super::Cmd::Quit).unwrap();
        assert!(!super::wait_bitperfect(&shared, &rx));
    }

    #[test]
    fn reorder_idx_pins_playing_track() {
        // moving the playing row itself follows it
        assert_eq!(reorder_idx(3, 3, 0), 0);
        assert_eq!(reorder_idx(3, 3, 7), 7);
        // move crossing from before to after: playing index shifts up
        assert_eq!(reorder_idx(3, 0, 5), 2);
        assert_eq!(reorder_idx(3, 0, 3), 2);
        // move crossing from after to before: shifts down
        assert_eq!(reorder_idx(3, 6, 0), 4);
        assert_eq!(reorder_idx(3, 6, 3), 4);
        // moves that do not cross the playing index leave it alone
        assert_eq!(reorder_idx(3, 0, 1), 3);
        assert_eq!(reorder_idx(3, 5, 7), 3);
        assert_eq!(reorder_idx(3, 0, 0), 3);
    }
}
