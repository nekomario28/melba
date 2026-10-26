//! Playback types shared between the interface and the daemon worker.
//!
//! [`LocalState`] is the playback snapshot the daemon reports and the UI
//! renders; [`PlayerCommand`] and [`LoadSpec`] are the commands sent to it.

use std::time::Instant;

use crate::api::models::ArtistRef;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playback {
    #[default]
    Stopped,
    Loading,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }

    pub fn api_name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }

    pub fn from_api(name: &str) -> Self {
        match name {
            "context" => Self::Context,
            "track" => Self::Track,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalTrack {
    pub uri: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub art_url: Option<String>,
    pub art_small_url: Option<String>,
    pub duration_ms: u32,
    pub is_episode: bool,
}

impl LocalTrack {
    pub fn artist_names(&self) -> String {
        crate::api::models::join_names(self.artists.iter().map(|artist| artist.name.as_str()))
    }
}

/// The source format the daemon actually opened. Lossless encoding and an
/// unmixed output route are separate facts: ALAC through a shared sink is
/// still ALAC, but does not establish bit-perfect output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamQuality {
    pub codec: String,
    pub sample_rate: u32,
    pub bits: u32,
    pub sink: String,
    pub verdict: String,
}

impl StreamQuality {
    pub fn label(&self) -> Option<String> {
        let codec = match self.codec.as_str() {
            "alac" => "ALAC · LOSSLESS",
            "mp4a" | "aac" => "AAC",
            _ => return None,
        };
        if self.sample_rate == 0 {
            return None;
        }
        let rate = self.sample_rate as f64 / 1000.0;
        // AAC is lossy: its decoder's PCM precision is not an encoded depth.
        let depth = if self.codec == "alac" && self.bits > 0 {
            format!(" · {}-bit", self.bits)
        } else {
            String::new()
        };
        Some(format!("{codec}{depth} · {rate} kHz"))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalState {
    pub playback: Playback,
    pub track: Option<LocalTrack>,
    pub position_ms: u32,
    /// When `position_ms` was observed; `None` while not advancing.
    pub position_at: Option<Instant>,
    pub volume: u16,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// The playback daemon's session is alive. Connect device
    /// activity is separate: Spotify may make this device inactive while the
    /// session remains ready to be activated by the next load.
    pub connected: bool,
    pub username: String,
    pub active_client: String,
    pub quality: Option<StreamQuality>,
    pub error: Option<String>,
    pub seek_sequence: u64,
    /// The daemon is fetching a track right now, even when the previous
    /// track's `playback` still reads `Playing` so its controls stay visible
    /// through the swap.
    pub loading: bool,
    /// A newly loaded track, including another play of the same URI.
    pub track_sequence: u64,
    /// Another play of the same track started, and its start position has
    /// not arrived yet. The track looks unchanged, so the `Playing` or
    /// `Paused` that brings the position counts as a seek for media
    /// controls, which would otherwise count on past the end (#587).
    pub replay_pending: bool,
}

/// What local playback was doing when its session ended, so the daemon
/// can pick it up again after reconnecting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interrupted {
    pub uri: String,
    pub position_ms: u32,
    /// Playing or loading, as opposed to paused.
    pub playing: bool,
}

impl LocalState {
    /// The track and position to come back to, if something was on.
    pub fn interrupted(&self) -> Option<Interrupted> {
        let track = self.track.as_ref()?;
        if self.playback == Playback::Stopped {
            return None;
        }
        Some(Interrupted {
            uri: track.uri.clone(),
            position_ms: self.position_now(),
            playing: matches!(self.playback, Playback::Playing | Playback::Loading),
        })
    }

    /// The position now, interpolated from the last report while playing.
    pub fn position_now(&self) -> u32 {
        match (self.playback, self.position_at) {
            (Playback::Playing, Some(at)) => {
                let elapsed = at.elapsed().as_millis() as u32;
                let limit = self
                    .track
                    .as_ref()
                    .map_or(u32::MAX, |track| track.duration_ms.max(self.position_ms));
                self.position_ms.saturating_add(elapsed).min(limit)
            }
            _ => self.position_ms,
        }
    }

    pub fn is_active(&self) -> bool {
        self.track.is_some() && self.playback != Playback::Stopped
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadSpec {
    pub context_uri: Option<String>,
    pub uris: Vec<String>,
    pub offset_uri: Option<String>,
    pub offset_index: Option<u32>,
    pub position_ms: u32,
    pub play: bool,
    pub shuffle: Option<bool>,
    /// Explicit repeat preference for a new load. Otherwise retain the
    /// daemon's current preference instead of its default (off).
    pub repeat: Option<RepeatMode>,
    /// Play what Spotify would follow `context_uri` with, its autoplay
    /// station, rather than the context itself.
    pub autoplay: bool,
}

/// How a daemon session picks playback back up.
#[derive(Clone, Debug)]
pub enum PlaybackResume {
    Track(LoadSpec),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    Toggle,
    Next,
    Previous,
    /// Remove manually queued tracks and keep context tracks.
    ClearQueue,
    /// Queue a track or episode after the ones already queued.
    AddToQueue(String),
    Seek(u32),
    /// The volume to keep: applied at once and told to Spotify Connect.
    Volume(u16),
    /// The slider mid-drag: applied at once, nothing sent. Every Connect
    /// update costs a round trip to the service, and the daemon makes them one
    /// after another, so dragging through fifty values lagged by seconds.
    VolumePreview(u16),
    Shuffle(bool),
    Repeat(RepeatMode),
    Load(LoadSpec),
}
