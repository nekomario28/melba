//! Backend worker that talks to the `melbad` daemon instead of Spotify.
//!
//! `Backend` exposes the same command/event channel surface regardless of
//! which worker runs behind it. This worker serves the slice the melbad daemon
//! daemon can serve today — transport, queue, local playback state, a
//! single "this computer" device — and drops commands with no Melba
//! equivalent, so the app's optimistic model keeps its own answer.
//!
//! Melba's D-Bus surface (org.mpris.MediaPlayer2.melba):
//!   org.mpris.MediaPlayer2.Player — Play/Pause/PlayPause/Next/Previous/
//!       Seek/SetPosition/OpenUri, props Volume/Shuffle/LoopStatus/
//!       Position/Metadata/PlaybackStatus
//!   MelbaEnqueue(as) MelbaGoTo(u) MelbaQueueReorder(u,u) MelbaQueueClear()
//!       MelbaLoadQueue(as,u,u,b) MelbaSetDevice(s)
//!   Metadata extras: melba:queue melba:titles melba:artists melba:artUrls
//!       melba:idx melba:codec melba:verdict melba:sink melba:device

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use dbus::arg::RefArg;
use dbus::blocking::Connection;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use tokio::sync::mpsc::UnboundedReceiver;

use std::sync::Arc;

use crate::api::ApiError;
use crate::api::models::{
    ArtistRef, AudioOutputs, CursorPage, Device, Image, PlayableItem, PlaybackState, Track, User,
};
use crate::backend::{ApiRequest, ApiResponse, AuthStatus, Command, Event, LocalPlayback};
use crate::mk::MusicKit;
use crate::player::{LoadSpec, LocalState, LocalTrack, Playback, PlayerCommand, RepeatMode};

const BUS: &str = "org.mpris.MediaPlayer2.melba";
const OBJ: &str = "/org/mpris/MediaPlayer2";
const IFACE: &str = "org.mpris.MediaPlayer2.Player";
const POLL: Duration = Duration::from_millis(500);
const CALL_TIMEOUT: Duration = Duration::from_millis(150);

/// What to do about the daemon this process owns at this tick.
#[derive(Debug, PartialEq, Eq)]
enum DaemonAction {
    /// Nothing to do: no owned daemon, or it is still running.
    Idle,
    /// The owned daemon exited and something else is serving MPRIS.
    Reap,
    /// The owned daemon exited and nothing is serving MPRIS: restart it.
    Restart,
    /// The owned daemon exited, nothing is serving, and restarts are
    /// exhausted or too soon.
    GiveUp,
}

/// How long to wait before restarting a daemon that died on its own.
const DAEMON_RESTART_COOLDOWN: Duration = Duration::from_secs(3);
/// Restarts allowed per app run before the app stops trying.
const DAEMON_RESTART_LIMIT: u32 = 3;

pub struct DaemonWorker {
    events: Sender<Event>,
    waker: crate::backend::Waker,
    conn: Option<Connection>,
    local: LocalState,
    /// Daemon queue rows parallel to `melba:queue` / `melba:titles` /
    /// `melba:artists` / `melba:artUrls`, plus the active row index.
    queue_uris: Vec<String>,
    queue_titles: Vec<String>,
    queue_artists: Vec<String>,
    queue_arts: Vec<String>,
    queue_idx: u32,
    /// Planned raw queue indices. None is an older daemon, not an empty plan.
    queue_next: Option<Vec<usize>>,
    /// `mpris:trackid`, needed by SetPosition.
    trackid: String,
    codec: String,
    device_name: String,
    emitted_ready: bool,
    authed: bool,
    auth_ticks: u8,
    mk: Arc<MusicKit>,
    update_proxy: crate::settings::ProxyConfig,
    sign_in: Option<crate::daemon_sign_in::SignIn>,
    current_attempt: u64,
    auth_epoch: Arc<std::sync::atomic::AtomicU64>,
    auth_events: Option<tokio::sync::mpsc::UnboundedSender<(u64, melba_auth::Event)>>,
    context_loads: Option<tokio::sync::mpsc::UnboundedSender<(u64, LoadSpec, Vec<String>)>>,
    load_epoch: u64,
    /// The daemon this process started, kept so its exit can be noticed and
    /// reaped. A child that dies unnoticed stays a zombie and leaves the app
    /// unable to play for as long as it runs.
    daemon: Option<std::process::Child>,
    /// When the daemon last exited, so a crash loop does not spin.
    daemon_lost_at: Option<Instant>,
    /// How many times this run has restarted a daemon that died on its own.
    daemon_restarts: u32,
}

pub(crate) fn daemon_command(executable: Option<std::path::PathBuf>) -> std::process::Command {
    // Cargo's bin directory need not be in the desktop session's PATH.
    let daemon = executable
        .and_then(|exe| exe.parent().map(|directory| directory.join("melbad")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| "melbad".into());
    let mut command = std::process::Command::new(daemon);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

fn sign_in_command(executable: Option<std::path::PathBuf>) -> std::process::Command {
    let mut command = daemon_command(executable);
    command.args(["auth", "gui"]);
    command
}

impl DaemonWorker {
    pub fn new(
        events: Sender<Event>,
        waker: crate::backend::Waker,
        auth_epoch: Arc<std::sync::atomic::AtomicU64>,
        update_proxy: crate::settings::ProxyConfig,
    ) -> Self {
        Self {
            events,
            waker,
            conn: Connection::new_session().ok(),
            local: LocalState::default(),
            queue_uris: Vec::new(),
            queue_titles: Vec::new(),
            queue_artists: Vec::new(),
            queue_arts: Vec::new(),
            queue_idx: 0,
            queue_next: None,
            trackid: String::new(),
            codec: String::new(),
            device_name: String::new(),
            emitted_ready: false,
            authed: false,
            auth_ticks: 0,
            mk: Arc::new(MusicKit::new(reqwest::Client::new())),
            update_proxy,
            sign_in: None,
            current_attempt: 0,
            auth_epoch,
            auth_events: None,
            context_loads: None,
            load_epoch: 0,
            daemon: None,
            daemon_lost_at: None,
            daemon_restarts: 0,
        }
    }

    fn emit(&self, event: Event) {
        if std::env::var_os("MELBA_WORKER_DEBUG").is_some() {
            eprintln!("melba-worker: emit {}", event_name(&event));
        }
        let _ = self.events.send(event);
        self.waker.wake();
    }

    fn proxy(&self) -> Option<dbus::blocking::Proxy<'_, &Connection>> {
        self.conn
            .as_ref()
            .map(|conn| conn.with_proxy(BUS, OBJ, CALL_TIMEOUT))
    }

    fn call(&self, method: &str, args: impl dbus::arg::AppendAll) -> Result<(), String> {
        let Some(proxy) = self.proxy() else {
            return Err("no session bus".into());
        };
        proxy
            .method_call::<(), _, _, _>(IFACE, method, args)
            .map_err(|e| e.to_string())
    }

    fn call_string(&self, method: &str, args: impl dbus::arg::AppendAll) -> Result<String, String> {
        let Some(proxy) = self.proxy() else {
            return Err("no session bus".into());
        };
        proxy
            .method_call::<(String,), _, _, _>(IFACE, method, args)
            .map(|(value,)| value)
            .map_err(|e| e.to_string())
    }

    fn set_prop<T: dbus::arg::Arg + dbus::arg::Append + 'static>(&self, name: &str, value: T) {
        if let Some(proxy) = self.proxy() {
            let _ = proxy.set(IFACE, name, value);
        }
    }

    /// The whole daemon snapshot; `None` while the daemon is unreachable.
    fn poll_daemon(&mut self) -> bool {
        let Some(proxy) = self.proxy() else {
            if std::env::var_os("MELBA_WORKER_DEBUG").is_some() {
                eprintln!("melba-worker: no session bus");
            }
            return false;
        };
        let status = proxy.get::<String>(IFACE, "PlaybackStatus");
        if std::env::var_os("MELBA_WORKER_DEBUG").is_some() {
            eprintln!("melba-worker: poll status={status:?}");
        }
        let Ok(status) = status else {
            return false;
        };
        let mut next = self.local.clone();
        next.playback = match status.as_str() {
            "Playing" => Playback::Playing,
            "Paused" => Playback::Paused,
            _ => Playback::Stopped,
        };
        next.shuffle = proxy.get::<bool>(IFACE, "Shuffle").unwrap_or(false);
        next.repeat = match proxy
            .get::<String>(IFACE, "LoopStatus")
            .unwrap_or_else(|_| "None".into())
            .as_str()
        {
            "Track" => RepeatMode::Track,
            "Playlist" => RepeatMode::Context,
            _ => RepeatMode::Off,
        };
        let pos_ms = (proxy.get::<i64>(IFACE, "Position").unwrap_or(0) / 1000).max(0) as u32;
        let advancing = next.position_ms != pos_ms || next.playback != self.local.playback;
        next.position_ms = pos_ms;
        next.position_at = (next.playback == Playback::Playing && advancing)
            .then(Instant::now)
            .or(if next.playback == Playback::Playing {
                Some(Instant::now())
            } else {
                None
            });
        next.volume = (proxy
            .get::<f64>(IFACE, "Volume")
            .unwrap_or(1.0)
            .clamp(0.0, 1.0)
            * f64::from(u16::MAX)) as u16;
        next.connected = true;
        next.username = "melbad".into();
        next.active_client = "melbad".into();

        if let Ok(meta) = proxy.get::<dbus::arg::PropMap>(IFACE, "Metadata") {
            let str_at = |key: &str| -> String {
                meta.get(key)
                    .and_then(|v| v.0.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let trackid = str_at("mpris:trackid");
            if trackid != self.trackid {
                next.track_sequence = next.track_sequence.wrapping_add(1);
            }
            self.trackid = trackid;
            self.codec = str_at("melba:codec");
            let number = |key: &str| {
                meta.get(key)
                    .and_then(|v| v.0.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0)
            };
            let quality = crate::player::StreamQuality {
                codec: self.codec.clone(),
                sample_rate: number("melba:sampleRate"),
                bits: number("melba:bitDepth"),
                sink: str_at("melba:sink"),
                verdict: str_at("melba:verdict"),
            };
            next.quality = quality.label().is_some().then_some(quality);
            self.device_name = str_at("melba:device");
            self.queue_idx = meta
                .get("melba:idx")
                .and_then(|v| v.0.as_u64())
                .unwrap_or(0) as u32;
            self.queue_next = meta
                .get("melba:nextIndices")
                .and_then(|v| v.0.as_iter())
                .map(|it| {
                    it.filter_map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
                        .collect()
                });
            if self.queue_next.is_none() && std::env::var_os("MELBA_WORKER_DEBUG").is_some() {
                eprintln!(
                    "melba-worker: melba:nextIndices absent; legacy queue order is not a shuffle prediction"
                );
            }
            for (key, dst) in [
                ("melba:queue", &mut self.queue_uris),
                ("melba:titles", &mut self.queue_titles),
                ("melba:artists", &mut self.queue_artists),
                ("melba:artUrls", &mut self.queue_arts),
            ] {
                *dst = meta
                    .get(key)
                    .and_then(|v| v.0.as_iter())
                    .map(|it| {
                        it.filter_map(|a| a.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            for uri in &mut self.queue_uris {
                *uri = model_uri(uri);
            }
            let uri = model_uri(&str_at("xesam:url"));
            let title = str_at("xesam:title");
            next.track = if title.is_empty() && uri.is_empty() {
                None
            } else {
                Some(LocalTrack {
                    uri,
                    title,
                    artists: meta
                        .get("xesam:artist")
                        .and_then(|v| v.0.as_iter())
                        .map(|it| {
                            it.filter_map(|a| {
                                a.as_str().map(|s| ArtistRef {
                                    name: s.to_string(),
                                    ..ArtistRef::default()
                                })
                            })
                            .collect()
                        })
                        .unwrap_or_default(),
                    album: str_at("xesam:album"),
                    art_url: Some(str_at("mpris:artUrl")).filter(|s| !s.is_empty()),
                    art_small_url: None,
                    duration_ms: (meta
                        .get("mpris:length")
                        .and_then(|v| v.0.as_i64())
                        .unwrap_or(0)
                        / 1000)
                        .max(0) as u32,
                    is_episode: false,
                })
            };
        }
        let changed = next != self.local;
        self.local = next;
        changed
    }

    fn handle_player(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Toggle => {
                let _ = self.call("PlayPause", ());
            }
            PlayerCommand::Next => {
                let _ = self.call("Next", ());
            }
            PlayerCommand::Previous => {
                let _ = self.call("Previous", ());
            }
            PlayerCommand::Seek(ms) => {
                if !self.trackid.is_empty() {
                    let _ = self.call(
                        "SetPosition",
                        (dbus::Path::from(self.trackid.clone()), i64::from(ms) * 1000),
                    );
                }
            }
            PlayerCommand::Volume(v) | PlayerCommand::VolumePreview(v) => {
                self.set_prop("Volume", f64::from(v) / f64::from(u16::MAX));
            }
            PlayerCommand::Shuffle(on) => self.set_prop("Shuffle", on),
            PlayerCommand::Repeat(mode) => self.set_prop(
                "LoopStatus",
                match mode {
                    RepeatMode::Track => "Track".to_string(),
                    RepeatMode::Context => "Playlist".to_string(),
                    RepeatMode::Off => "None".to_string(),
                },
            ),
            PlayerCommand::ClearQueue => {
                let _ = self.call("MelbaQueueClear", ());
            }
            PlayerCommand::AddToQueue(uri) => {
                let _ = self.call("MelbaEnqueue", (vec![daemon_uri(&uri)],));
            }
            PlayerCommand::Load(spec) => {
                self.load_epoch = self.load_epoch.wrapping_add(1);
                if spec.uris.is_empty() && spec.context_uri.is_some() {
                    self.spawn_context_load(spec);
                } else {
                    self.load(spec);
                }
            }
        }
    }

    /// Replace the complete context and select its row in one daemon command.
    /// OpenUri + enqueue loses ordering, accumulates old contexts, and races seeks.
    fn load(&self, spec: LoadSpec) {
        let LoadSpec {
            uris,
            offset_uri,
            offset_index,
            position_ms,
            play,
            shuffle,
            repeat,
            ..
        } = spec;
        if uris.is_empty() {
            return;
        }
        let selected = offset_uri.as_deref().map(daemon_uri);
        // An explicit row disambiguates duplicate URIs. When context resolution
        // changed the list, use the named track instead of a stale row number.
        let start = offset_index
            .map(|i| i as usize)
            .filter(|i| {
                uris.get(*i).is_some_and(|u| {
                    selected
                        .as_ref()
                        .is_none_or(|wanted| daemon_uri(u) == *wanted)
                })
            })
            .or_else(|| {
                selected
                    .as_ref()
                    .and_then(|uri| uris.iter().position(|u| daemon_uri(u) == *uri))
            })
            .or_else(|| (selected.is_none() && offset_index.is_none()).then_some(0));
        let Some(start) = start else {
            self.emit(Event::Error(
                "The selected track is not in the playback context.".into(),
            ));
            return;
        };
        if let Some(on) = shuffle {
            self.set_prop("Shuffle", on);
        }
        if let Some(mode) = repeat {
            self.set_prop(
                "LoopStatus",
                match mode {
                    RepeatMode::Track => "Track".to_string(),
                    RepeatMode::Context => "Playlist".to_string(),
                    RepeatMode::Off => "None".to_string(),
                },
            );
        }
        let uris: Vec<String> = uris.iter().map(|u| daemon_uri(u)).collect();
        if self
            .call("MelbaLoadQueue", (uris, start as u32, position_ms, play))
            .is_err()
        {
            self.emit(Event::Error("The playback request was rejected. If the app was updated, restart the playback daemon.".into()));
        }
    }

    /// Resolve context tracks off the worker, keeping the original selection
    /// and playback options. Only the worker sends playback commands, so a
    /// slow catalog answer cannot supersede a newer play request.
    fn spawn_context_load(&self, spec: LoadSpec) {
        let Some(ctx) = spec.context_uri.clone() else {
            return;
        };
        let mk = self.mk.clone();
        let Some(completed) = self.context_loads.clone() else {
            return;
        };
        let epoch = self.load_epoch;
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                resolve_context(&ctx, |request| catalog_answer(&mk, request)),
            )
            .await;
            match result {
                Ok(Ok(uris)) => {
                    let _ = completed.send((epoch, spec, uris));
                }
                Ok(Err(reason)) => log::warn!("playback context resolution failed: {reason}"),
                Err(_) => log::warn!("playback context resolution timed out"),
            }
        });
    }

    fn context_resolved(&self, epoch: u64, mut spec: LoadSpec, uris: Vec<String>) {
        if epoch != self.load_epoch {
            return;
        }
        spec.uris = uris;
        self.load(spec);
    }

    fn queue_track(&self, i: usize) -> Option<PlayableItem> {
        let uri = self.queue_uris.get(i)?.clone();
        Some(PlayableItem::Track(Track {
            id: uri.rsplit(':').next().map(str::to_string),
            uri,
            name: self.queue_titles.get(i).cloned().unwrap_or_default(),
            artists: self
                .queue_artists
                .get(i)
                .map(|names| {
                    names
                        .split(", ")
                        .map(|name| ArtistRef {
                            name: name.to_string(),
                            ..ArtistRef::default()
                        })
                        .collect()
                })
                .unwrap_or_default(),
            album: None,
            ..Track::default()
        }))
    }

    fn playback_state(&self) -> PlaybackState {
        PlaybackState {
            device: Some(self.device()),
            repeat_state: match self.local.repeat {
                RepeatMode::Track => "track".into(),
                RepeatMode::Context => "context".into(),
                RepeatMode::Off => "off".into(),
            },
            shuffle_state: self.local.shuffle,
            timestamp: 0,
            progress_ms: Some(self.local.position_now()),
            is_playing: self.local.playback == Playback::Playing,
            item: self.local.track.as_ref().map(|track| {
                PlayableItem::Track(Track {
                    uri: track.uri.clone(),
                    name: track.title.clone(),
                    duration_ms: track.duration_ms,
                    artists: track.artists.clone(),
                    ..Track::default()
                })
            }),
            currently_playing_type: Some("track".into()),
            ..PlaybackState::default()
        }
    }

    fn device(&self) -> Device {
        Device {
            id: Some("melbad".into()),
            name: if self.device_name.is_empty() || self.device_name == "auto" {
                "This computer".into()
            } else {
                format!("This computer ({})", self.device_name)
            },
            is_active: true,
            volume_percent: Some(crate::app::volume_to_percent(self.local.volume)),
            supports_volume: Some(true),
            kind: "Computer".into(),
            ..Device::default()
        }
    }

    fn handle_api(&self, request: ApiRequest) {
        let response = match request {
            ApiRequest::Me => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                let mk = Arc::clone(&self.mk);
                tokio::spawn(async move {
                    let user = profile(&mk).await.unwrap_or_else(|| User {
                        id: "apple-music".into(),
                        display_name: Some("Apple Music".into()),
                        product: Some("premium".into()),
                        uri: Some("melba:user:apple-music".into()),
                        ..User::default()
                    });
                    let _ = events.send(Event::Api(Box::new(ApiResponse::Me(Ok(user)))));
                    waker.wake();
                });
                None
            }
            ApiRequest::Devices => Some(ApiResponse::Devices(Ok(vec![self.device()]))),
            ApiRequest::AudioDevices => {
                let result = self
                    .call_string("MelbaAudioDevices", ())
                    .and_then(|json| {
                        serde_json::from_str::<AudioOutputs>(&json)
                            .map_err(|e| format!("audio devices: {e}"))
                    })
                    .map_err(ApiError::Decode);
                Some(ApiResponse::AudioDevices(result))
            }
            ApiRequest::SetAudioDevice { device } => {
                let result = self
                    .call("MelbaSetAudioDevice", (device,))
                    .map_err(ApiError::Network);
                Some(ApiResponse::AudioDeviceSet(result))
            }
            ApiRequest::PlaybackState { seq } => Some(ApiResponse::PlaybackState {
                seq,
                result: Ok(Some(self.playback_state())),
            }),
            ApiRequest::Queue { seq } => Some(ApiResponse::Queue {
                seq,
                result: Ok(crate::api::models::Queue {
                    currently_playing: self.queue_track(self.queue_idx as usize),
                    queue: self
                        .queue_next
                        .clone()
                        .unwrap_or_else(|| {
                            ((self.queue_idx as usize + 1)..self.queue_uris.len()).collect()
                        })
                        .into_iter()
                        .filter_map(|i| self.queue_track(i))
                        .collect(),
                }),
            }),
            ApiRequest::AddToQueue { uri, label, .. } => {
                let result = self
                    .call("MelbaEnqueue", (vec![daemon_uri(&uri)],))
                    .map_err(ApiError::Network);
                Some(ApiResponse::QueueAdded { label, result })
            }
            ApiRequest::AddManyToQueue { request, uris, .. } => {
                let mapped: Vec<String> = uris.iter().map(|u| daemon_uri(u)).collect();
                let added = mapped.len();
                let result = self
                    .call("MelbaEnqueue", (mapped,))
                    .map_err(ApiError::Network);
                Some(ApiResponse::QueueBatchAdded {
                    request,
                    added,
                    result,
                })
            }
            ApiRequest::RecentlyPlayed {
                who,
                generation,
                limit,
                ..
            } => {
                let events = self.events.clone();
                let waker = self.waker.clone();
                let mk = Arc::clone(&self.mk);
                tokio::spawn(async move {
                    let result = recent(&mk, limit).await;
                    let _ = events.send(Event::Api(Box::new(ApiResponse::RecentlyPlayed {
                        who,
                        generation,
                        limit,
                        result,
                    })));
                    waker.wake();
                });
                None
            }
            request => {
                self.spawn_catalog(request);
                None
            }
        };
        if let Some(response) = response {
            self.emit(Event::Api(Box::new(response)));
        }
    }

    /// Catalog reads go through MusicKit on the runtime; daemon-backed
    /// requests above answered synchronously. Unhandled variants return
    /// `None` and the app's optimistic model keeps its answer.
    fn spawn_catalog(&self, request: ApiRequest) {
        let events = self.events.clone();
        let waker = self.waker.clone();
        let mk = Arc::clone(&self.mk);
        tokio::spawn(async move {
            if let Some(response) = catalog_answer(&mk, request).await {
                let _ = events.send(Event::Api(Box::new(response)));
                waker.wake();
            }
        });
    }

    /// Every attempt has its own CLI, private pipes, and generation. Old
    /// events never inspect the token file or complete a newer attempt.
    fn spawn_sign_in(&mut self, attempt: u64, restore: bool) {
        if self.auth_epoch.load(std::sync::atomic::Ordering::Acquire) != attempt {
            return;
        }
        if let Some(job) = self.sign_in.take() {
            job.cancel();
        }
        if self.auth_epoch.load(std::sync::atomic::Ordering::Acquire) != attempt {
            return;
        }
        self.current_attempt = attempt;
        let mut command = sign_in_command(std::env::current_exe().ok());
        if restore {
            command.arg("--restore");
        }
        self.authed = false;
        self.emit(Event::AuthAttempt {
            attempt,
            status: AuthStatus::Connecting,
        });
        if let Some(events) = &self.auth_events {
            self.sign_in = Some(crate::daemon_sign_in::SignIn::start(
                attempt,
                restore,
                command,
                events.clone(),
            ));
        }
    }

    fn authentication_event(&mut self, attempt: u64, event: melba_auth::Event) {
        if self.auth_epoch.load(std::sync::atomic::Ordering::Acquire) != attempt
            || !self.sign_in.as_ref().is_some_and(|job| job.id == attempt)
        {
            return;
        }
        let status = match event {
            melba_auth::Event::Prompt { prompt } => {
                // Native forms are transient, including in the GUI process.
                // Do not let a later crash dump preserve their input buffers.
                if unsafe {
                    libc::prctl(
                        libc::PR_SET_DUMPABLE,
                        0 as libc::c_ulong,
                        0 as libc::c_ulong,
                        0 as libc::c_ulong,
                        0 as libc::c_ulong,
                    )
                } != 0
                {
                    if let Some(job) = self.sign_in.take() {
                        job.cancel();
                    }
                    AuthStatus::Failed("Cannot protect transient sign-in input.".into())
                } else {
                    AuthStatus::Prompt { attempt, prompt }
                }
            }
            melba_auth::Event::Browser { url } => AuthStatus::WaitingForBrowser { url },
            melba_auth::Event::Checking => AuthStatus::Connecting,
            melba_auth::Event::ReadyToCommit => {
                if self
                    .sign_in
                    .as_ref()
                    .is_some_and(|job| job.send(melba_auth::Input::Commit))
                {
                    return;
                }
                if let Some(job) = self.sign_in.take() {
                    job.cancel();
                }
                AuthStatus::Failed("Sign-in input channel closed.".into())
            }
            melba_auth::Event::Complete => {
                self.sign_in.take();
                self.authed = true;
                AuthStatus::Connected {
                    username: "Apple Music".into(),
                }
            }
            melba_auth::Event::SignedOut => {
                self.sign_in.take();
                AuthStatus::SignedOut
            }
            melba_auth::Event::Failed { message } => {
                self.sign_in.take();
                AuthStatus::Failed(message)
            }
        };
        self.emit(Event::AuthAttempt { attempt, status });
    }

    /// `melba:track:<id>` → MusicKit TTML lyrics on amp-api-edge.
    /// `Ok(None)` when the song has none.
    fn spawn_lyrics(&self, request: crate::backend::LyricsRequest) {
        let events = self.events.clone();
        let waker = self.waker.clone();
        let mk = Arc::clone(&self.mk);
        tokio::spawn(async move {
            let id = request
                .uri
                .strip_prefix("melba:track:")
                .or_else(|| request.uri.strip_prefix("spotify:track:"))
                .unwrap_or(&request.uri)
                .to_string();
            let result = mk.lyrics(&id).await;
            let _ = events.send(Event::Lyrics {
                uri: request.uri,
                result,
            });
            waker.wake();
        });
    }

    fn sign_out(&self) {
        let _ = self.call("Stop", ());
        let _ = self.call("MelbaStopManagedWrapper", ());
        if let Some(path) = MusicKit::token_path() {
            let _ = std::fs::remove_file(path);
        }
        self.emit(Event::Auth(AuthStatus::SignedOut));
    }

    fn restore_session(&mut self, enabled: bool) {
        if enabled {
            self.spawn_sign_in(0, true);
        }
    }

    fn update_task(
        &self,
        task: impl FnOnce(anyhow::Result<crate::updates::Updater>, &dyn Fn(Event)) + Send + 'static,
    ) {
        let proxy = self.update_proxy.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            let emit = |event| {
                let _ = events.send(event);
                waker.wake();
            };
            task(crate::updates::updater(&proxy), &emit);
        });
    }

    /// How long to wait before restarting a daemon that died on its own.
    const DAEMON_RESTART_COOLDOWN: Duration = Duration::from_secs(3);
    /// Restarts allowed per app run before the app stops trying.
    const DAEMON_RESTART_LIMIT: u32 = 3;

    fn daemon_action(
        owns_child: bool,
        child_exited: bool,
        daemon_available: bool,
        since_lost: Option<Duration>,
        restarts: u32,
    ) -> DaemonAction {
        if !owns_child || !child_exited {
            return DaemonAction::Idle;
        }
        if daemon_available {
            // Another daemon answers: reap ours and leave theirs alone.
            return DaemonAction::Reap;
        }
        if restarts >= DAEMON_RESTART_LIMIT {
            return DaemonAction::GiveUp;
        }
        if since_lost.is_some_and(|elapsed| elapsed < DAEMON_RESTART_COOLDOWN) {
            return DaemonAction::GiveUp;
        }
        DaemonAction::Restart
    }

    /// Reap and, when nothing else is serving MPRIS, restart the daemon we
    /// started. Without this a dead daemon is a zombie the app never notices,
    /// and "The playback request was rejected" is all the user ever sees.
    fn supervise_daemon(&mut self) {
        let child_exited = self
            .daemon
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
        if !child_exited {
            return;
        }
        // Always reap first: the process table entry is ours to clear.
        if let Some(mut child) = self.daemon.take() {
            let _ = child.wait();
        }
        self.daemon_lost_at = Some(Instant::now());
        match Self::daemon_action(
            true,
            true,
            self.daemon_available(),
            Some(Duration::ZERO),
            self.daemon_restarts,
        ) {
            DaemonAction::Reap | DaemonAction::Idle => {}
            DaemonAction::Restart => {
                self.daemon_restarts += 1;
                if let Some(child) = self.start_daemon() {
                    log::warn!(
                        "the playback daemon exited; restarted it ({} of {})",
                        self.daemon_restarts,
                        DAEMON_RESTART_LIMIT
                    );
                    self.daemon = Some(child);
                }
            }
            DaemonAction::GiveUp => {
                if self.daemon_restarts >= DAEMON_RESTART_LIMIT {
                    self.emit(Event::Error(
                        "The playback daemon keeps exiting. Reinstall Melba and melbad together."
                            .into(),
                    ));
                }
            }
        }
    }

    fn daemon_available(&self) -> bool {
        self.proxy()
            .and_then(|proxy| proxy.get::<dbus::arg::PropMap>(IFACE, "Metadata").ok())
            .is_some_and(|metadata| metadata.contains_key("melba:idx"))
    }

    /// Runs only in a real desktop session, on the backend thread. Keep the
    /// child handle so a failed start or shutdown cannot orphan our process.
    fn start_daemon(&self) -> Option<std::process::Child> {
        let mut command = daemon_command(std::env::current_exe().ok());
        command.args(["serve", "--paused"]);
        self.start_daemon_command(command)
    }

    fn start_daemon_command(
        &self,
        mut command: std::process::Command,
    ) -> Option<std::process::Child> {
        if self.conn.is_none() || self.daemon_available() {
            return None;
        }
        if self.mpris_owner().is_some() {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if self.daemon_available() {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            self.emit(Event::Error(
                "The media-control owner is running but the playback daemon is not responding."
                    .into(),
            ));
            return None;
        }
        let Ok(mut child) = command.spawn() else {
            self.emit(Event::Error(
                "The playback daemon could not start. Reinstall Melba and melbad together.".into(),
            ));
            return None;
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while std::time::Instant::now() < deadline {
            if self.daemon_available() {
                return Some(child);
            }
            if !matches!(child.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        self.emit(Event::Error(
            "The playback daemon did not become ready.".into(),
        ));
        None
    }

    fn mpris_owner(&self) -> Option<String> {
        self.conn
            .as_ref()?
            .with_proxy(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                CALL_TIMEOUT,
            )
            .method_call::<(String,), _, _, _>("org.freedesktop.DBus", "GetNameOwner", (BUS,))
            .ok()
            .map(|reply| reply.0)
    }

    fn stop_daemon(&self, mut child: Option<std::process::Child>) {
        let deadline = Instant::now() + Duration::from_secs(4);
        if let (Some(connection), Some(owner)) = (&self.conn, self.mpris_owner()) {
            // Pin the unique owner: a replacement frontend must never receive
            // a Quit meant for the daemon we verified.
            let remote = connection.with_proxy(owner.as_str(), OBJ, CALL_TIMEOUT);
            let bus = connection.with_proxy(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                CALL_TIMEOUT,
            );
            let mut sent = false;
            while Instant::now() < deadline {
                let present: Result<(bool,), _> =
                    bus.method_call("org.freedesktop.DBus", "NameHasOwner", (owner.as_str(),));
                if matches!(present, Ok((false,))) {
                    break;
                }
                if !sent && let Ok(metadata) = remote.get::<dbus::arg::PropMap>(IFACE, "Metadata") {
                    // A successful foreign-player probe is not a slow daemon.
                    if !metadata.contains_key("melba:idx") {
                        break;
                    }
                    let result: Result<(), _> =
                        remote.method_call("org.mpris.MediaPlayer2", "Quit", ());
                    sent = result.is_ok();
                }
                // A timeout is inconclusive. In particular OpenUri forwarding
                // can occupy the daemon handler for two seconds.
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        if let Some(child) = &mut child {
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if self.daemon_available() {
            log::warn!("playback daemon is still running after shutdown");
        }
    }

    pub async fn run(
        mut self,
        mut commands: UnboundedReceiver<Command>,
        restore_sign_in: bool,
        manage_daemon: bool,
    ) {
        let owned_daemon = manage_daemon.then(|| self.start_daemon()).flatten();
        let daemon_managed = owned_daemon.is_some();
        self.daemon = owned_daemon;
        let (auth_events, mut auth_results) = tokio::sync::mpsc::unbounded_channel();
        self.auth_events = Some(auth_events);
        let (completed, mut context_results) = tokio::sync::mpsc::unbounded_channel();
        self.context_loads = Some(completed);
        self.restore_session(restore_sign_in);
        let mut interval = tokio::time::interval(POLL);
        loop {
            tokio::select! {
                Some((attempt, event)) = auth_results.recv() => self.authentication_event(attempt, event),
                Some((epoch, spec, uris)) = context_results.recv() => self.context_resolved(epoch, spec, uris),
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    if std::env::var_os("MELBA_WORKER_DEBUG").is_some() {
                        eprintln!("melba-worker: command {}", command_name(&command));
                    }
                    match command {
                        Command::Shutdown => break,
                        Command::Player(player) => self.handle_player(player),
                        Command::Api(request) => self.handle_api(request),
                        Command::AuthStart {attempt,restore} => self.spawn_sign_in(attempt,restore),
                        Command::Reconnect => {
                            use std::sync::atomic::Ordering;
                            let next = self.current_attempt.wrapping_add(1);
                            if self.authed && self.auth_epoch.compare_exchange(self.current_attempt,next,Ordering::AcqRel,Ordering::Acquire).is_ok() { self.spawn_sign_in(next,true); }
                        },
                        Command::AuthInput { attempt, input } => {
                            if let Some(job) = self.sign_in.as_ref().filter(|job| job.id == attempt && self.auth_epoch.load(std::sync::atomic::Ordering::Acquire) == attempt)
                                && !job.send(input) {
                                    if let Some(job) = self.sign_in.take() { job.cancel(); }
                                    self.emit(Event::AuthAttempt {attempt,status:AuthStatus::Failed("Sign-in input channel closed.".into())});
                            }
                        }
                        Command::CancelSignIn => {
                            self.authed = false;
                            if let Some(job) = self.sign_in.take() { job.cancel(); }
                            self.emit(Event::Auth(AuthStatus::SignedOut));
                        }
                        Command::SignOut => {
                            self.authed = false;
                            if let Some(job) = self.sign_in.take() { job.cancel(); }
                            self.sign_out();
                        }
                        Command::Lyrics(request) => self.spawn_lyrics(*request),
                        // This port ships with Melba, not through melba's
                        // release channel — there is nothing to download.
                        Command::UpdateProxy(proxy) => self.update_proxy = proxy,
                        Command::CheckForUpdates { manual, source } => {
                            if !restore_sign_in && source.is_github() {
                                self.emit(Event::UpdateChecked { manual, result: Err("Public update checks are disabled in an isolated demo session.".into()) });
                                continue;
                            }
                            self.update_task(move |updater, emit| {
                                let result = updater.and_then(|u| u.with_source(source).check()).map_err(|e| format!("{e:#}"));
                                emit(Event::UpdateChecked { manual, result });
                            });
                        }
                        Command::InspectUpdate => {
                            self.update_task(|updater, emit| {
                                let result = updater.map_err(|e| e.to_string()).and_then(|u| u.installation().map_err(|e| e.to_string()));
                                emit(Event::UpdateSupport(result));
                            });
                        }
                        Command::DownloadUpdate { release, source } => {
                            self.update_task(move |updater, emit| {
                                let result = updater.and_then(|u| {
                                    crate::updates::require_matching_daemon_version(&release.version)?;
                                    u.with_source(source).download(&release, |received, total| emit(Event::UpdateProgress { received, total }))
                                }).map(Box::new).map_err(|e| format!("{e:#}"));
                                emit(Event::UpdateDownloaded(result));
                            });
                        }
                        Command::InstallUpdate { prepared, arguments } => {
                            self.update_task(move |updater, emit| {
                                let result = updater.and_then(|u| {
                                    crate::updates::require_matching_daemon_version(prepared.version())?;
                                    u.handoff(*prepared, arguments)
                                }).map_err(|e| format!("{e:#}"));
                                emit(Event::UpdateInstalling(result));
                            });
                        }
                        _ => {}
                    }
                }
                _ = interval.tick() => {
                    if daemon_managed {
                        self.supervise_daemon();
                    }
                    if self.poll_daemon() {
                        self.emit(Event::Local(Box::new(self.local.clone())));
                    }
                    // Removal may sign OUT. A saved/old token may never sign
                    // IN while another native attempt is pending or cancelled.
                    self.auth_ticks = self.auth_ticks.wrapping_add(1);
                    if self.auth_ticks.is_multiple_of(16) && self.authed && MusicKit::user_token().is_none() {
                        self.authed = false;
                        self.emit(Event::Auth(AuthStatus::SignedOut));
                    }
                    if !self.emitted_ready {
                        self.emitted_ready = true;
                        let reachable = self.local.connected;
                        self.emit(Event::Playback(if reachable {
                            LocalPlayback::Ready {
                                device_id: "melbad".into(),
                            }
                        } else {
                            LocalPlayback::Unavailable
                        }));
                        if reachable {
                            self.emit(Event::Local(Box::new(self.local.clone())));
                        }
                    }
                }
            }
        }
        if let Some(job) = self.sign_in.take() {
            job.cancel();
        }
        if manage_daemon {
            let owned = self.daemon.take();
            self.stop_daemon(owned);
        }
    }
}

/// A search result group → a Page of mapped models.
fn page_of<T>(
    v: &serde_json::Value,
    kind: &str,
    map: impl Fn(&serde_json::Value) -> T,
) -> Option<crate::api::models::Page<T>> {
    let data = v["results"][kind]["data"].as_array()?;
    Some(crate::api::models::Page {
        items: data.iter().map(map).collect(),
        total: data.len() as u32,
        limit: 25,
        offset: 0,
        next: None,
    })
}

/// MusicKit-backed answers for catalog requests. `None` = unhandled; the
/// app keeps its optimistic answer rather than an error toast.
async fn catalog_answer(mk: &MusicKit, request: ApiRequest) -> Option<ApiResponse> {
    use crate::api::models::{Page, PlaylistItem, SavedAlbum, SavedTrack, SearchResults};

    let err = |e: String| ApiError::Network(e);
    let for_queue = matches!(request, ApiRequest::AlbumQueueTracks { .. });
    match request {
        ApiRequest::Search { query, serial } => {
            let result = mk
                .get(
                    "/search",
                    &[
                        ("term", &query),
                        ("types", "songs,artists,albums,playlists"),
                        ("limit", "25"),
                    ],
                )
                .await
                .map(|v| SearchResults {
                    tracks: page_of(&v, "songs", crate::mk::song),
                    artists: page_of(&v, "artists", crate::mk::artist),
                    albums: page_of(&v, "albums", crate::mk::album),
                    playlists: page_of(&v, "playlists", crate::mk::playlist),
                })
                .map_err(err);
            Some(ApiResponse::Search {
                query,
                serial,
                result,
            })
        }
        ApiRequest::PlaySearch {
            kind,
            query,
            serial,
        } => {
            let types = match kind.as_str() {
                "album" => "albums",
                "artist" => "artists",
                "playlist" => "playlists",
                _ => "songs",
            };
            let result = mk
                .get(
                    "/search",
                    &[("term", &query), ("types", types), ("limit", "1")],
                )
                .await
                .map(|v| {
                    let key = match types {
                        "songs" => "track",
                        other => &other[..other.len() - 1],
                    };
                    v["results"][types]["data"][0]["id"]
                        .as_str()
                        .map(|id| format!("melba:{key}:{id}"))
                })
                .map_err(err);
            Some(ApiResponse::PlaySearch {
                query,
                serial,
                result,
            })
        }
        ApiRequest::Track { id } => {
            let result = mk
                .get(&format!("/songs/{id}"), &[])
                .await
                .and_then(|v| {
                    v["data"][0]
                        .as_object()
                        .map(|_| crate::mk::song(&v["data"][0]))
                        .ok_or_else(|| format!("song {id} not found"))
                })
                .map_err(err);
            Some(ApiResponse::Track { id, result })
        }
        ApiRequest::Album { id } => {
            let result = mk
                .get(&format!("/albums/{id}"), &[])
                .await
                .map(|v| crate::mk::album(&v["data"][0]))
                .map_err(err);
            Some(ApiResponse::Album { id, result })
        }
        ApiRequest::AlbumTracks {
            id,
            offset,
            generation,
        }
        | ApiRequest::AlbumQueueTracks {
            id,
            offset,
            request: generation,
        } => {
            let result = mk
                .get(&format!("/albums/{id}"), &[("extend", "audioVariants")])
                .await
                .map(|v| {
                    let tracks = v["data"][0]["relationships"]["tracks"]["data"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let total = tracks.len() as u32;
                    let items: Vec<Track> = tracks
                        .iter()
                        .skip(offset as usize)
                        .map(crate::mk::song)
                        .collect();
                    Page {
                        items,
                        total,
                        limit: total,
                        offset,
                        next: None,
                    }
                })
                .map_err(err);
            Some(if for_queue {
                ApiResponse::AlbumQueueTracks {
                    offset,
                    request: generation,
                    result,
                }
            } else {
                ApiResponse::AlbumTracks {
                    id,
                    offset,
                    generation,
                    result,
                }
            })
        }
        ApiRequest::Artist { id } => {
            let result = mk
                .get(&format!("/artists/{id}"), &[])
                .await
                .map(|v| crate::mk::artist(&v["data"][0]))
                .map_err(err);
            Some(ApiResponse::Artist { id, result })
        }
        ApiRequest::ArtistTopTracks { id } => {
            let result = mk
                .get(&format!("/artists/{id}"), &[("views", "top-songs")])
                .await
                .map(|v| {
                    v["data"][0]["views"]["top-songs"]["data"]
                        .as_array()
                        .map(|data| data.iter().map(crate::mk::song).collect())
                        .unwrap_or_default()
                })
                .map_err(err);
            Some(ApiResponse::ArtistTopTracks { id, result })
        }
        ApiRequest::ArtistAlbums { id, groups, offset } => {
            let result = mk
                .get(&format!("/artists/{id}"), &[("views", "full-albums")])
                .await
                .map(|v| {
                    let albums: Vec<crate::api::models::Album> =
                        v["data"][0]["views"]["full-albums"]["data"]
                            .as_array()
                            .map(|data| data.iter().map(crate::mk::album).collect())
                            .unwrap_or_default();
                    Page {
                        total: albums.len() as u32,
                        limit: albums.len() as u32,
                        offset,
                        items: albums,
                        next: None,
                    }
                })
                .map_err(err);
            Some(ApiResponse::ArtistAlbums {
                id,
                groups,
                offset,
                result,
            })
        }
        ApiRequest::RelatedArtists { id } => Some(ApiResponse::RelatedArtists {
            id,
            result: Ok(Vec::new()),
        }),
        ApiRequest::Playlist { id, generation } => {
            // p.* = library playlist (own collection), pl.* = catalog.
            let result = if id.starts_with("p.") {
                mk.me_get(&format!("/library/playlists/{id}"), &[])
                    .await
                    .map(|v| crate::mk::playlist(&v["data"][0]))
            } else {
                mk.get(&format!("/playlists/{id}"), &[("extend", "audioVariants")])
                    .await
                    .map(|v| crate::mk::playlist(&v["data"][0]))
            }
            .map_err(err);
            Some(ApiResponse::Playlist {
                id,
                generation,
                result,
            })
        }
        ApiRequest::PlaylistItems {
            id,
            offset,
            generation,
        } => {
            // Catalog playlists embed the whole track list (slice here);
            // library playlists paginate server-side at 100 per page.
            let result = async {
                if id.starts_with("p.") {
                    let off = offset.to_string();
                    mk.me_get(
                        &format!("/library/playlists/{id}/tracks"),
                        &[("limit", "100"), ("offset", off.as_str())],
                    )
                    .await
                    .map(|v| {
                        (
                            v["data"].as_array().cloned().unwrap_or_default(),
                            v["next"].as_str().map(str::to_string),
                            v["meta"]["total"].as_u64().map(|n| n as u32),
                        )
                    })
                } else {
                    mk.get(&format!("/playlists/{id}"), &[("extend", "audioVariants")])
                        .await
                        .map(|v| {
                            let tracks = v["data"][0]["relationships"]["tracks"]["data"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            let total = tracks.len() as u32;
                            (
                                tracks.into_iter().skip(offset as usize).collect(),
                                None,
                                Some(total),
                            )
                        })
                }
            }
            .await
            .map(|(tracks, next, total)| {
                let items: Vec<PlaylistItem> = tracks
                    .iter()
                    .map(|d| PlaylistItem {
                        item: Some(PlayableItem::Track(crate::mk::song(d))),
                        ..PlaylistItem::default()
                    })
                    .collect();
                Page {
                    items,
                    total: total.unwrap_or(0),
                    limit: 100,
                    offset,
                    next,
                }
            })
            .map_err(err);
            Some(ApiResponse::PlaylistItems {
                id,
                offset,
                generation,
                result,
            })
        }
        ApiRequest::MyPlaylists { offset, generation } => {
            let result = mk
                .me_get("/library/playlists", &[("limit", "50")])
                .await
                .map(|v| Page {
                    items: v["data"]
                        .as_array()
                        .map(|data| data.iter().map(crate::mk::playlist).collect())
                        .unwrap_or_default(),
                    total: v["meta"]["total"].as_u64().unwrap_or(0) as u32,
                    limit: 50,
                    offset,
                    next: v["next"].as_str().map(str::to_string),
                })
                .map_err(err);
            Some(ApiResponse::MyPlaylists {
                offset,
                generation,
                result,
            })
        }
        ApiRequest::MadeForYou { generation, .. } => {
            let result = mk
                .me_get("/recommendations", &[("limit", "10")])
                .await
                .map(|v| {
                    v["data"]
                        .as_array()
                        .map(|recs| {
                            recs.iter()
                                .flat_map(|rec| {
                                    rec["relationships"]["contents"]["data"]
                                        .as_array()
                                        .cloned()
                                        .unwrap_or_default()
                                })
                                .filter(|d| d["type"].as_str() == Some("playlists"))
                                .map(|d| crate::mk::playlist(&d))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .map_err(err);
            Some(ApiResponse::MadeForYou {
                generation,
                personalized: true,
                result,
            })
        }
        ApiRequest::Contains { uris } => Some(ApiResponse::Contains {
            result: mk.contains(&uris).await.map_err(err),
            uris,
        }),
        ApiRequest::SetSaved { uris, saved } => {
            let result = if saved {
                mk.save(&uris).await
            } else {
                mk.unsave(&uris).await
            }
            .map_err(err);
            Some(ApiResponse::SavedChanged {
                uris,
                saved,
                result,
            })
        }
        // Apple Music library: the user's own collection under /v1/me/library.
        ApiRequest::SavedTracks {
            offset,
            generation,
            account_id,
        } => {
            let result = mk
                .me_get(
                    "/library/songs",
                    &[("limit", "100"), ("offset", &offset.to_string())],
                )
                .await
                .map(|v| {
                    let items = v["data"]
                        .as_array()
                        .map(|data| {
                            data.iter()
                                .map(|d| SavedTrack {
                                    added_at: d["attributes"]["dateAdded"]
                                        .as_str()
                                        .map(str::to_string),
                                    track: crate::mk::song(d),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Page {
                        total: v["meta"]["total"].as_u64().unwrap_or(0) as u32,
                        limit: 100,
                        offset,
                        items,
                        next: v["next"].as_str().map(str::to_string),
                    }
                })
                .map_err(err);
            Some(ApiResponse::SavedTracks {
                offset,
                generation,
                account_id,
                result,
            })
        }
        ApiRequest::SavedAlbums { offset } => {
            let result = mk
                .me_get(
                    "/library/albums",
                    &[("limit", "100"), ("offset", &offset.to_string())],
                )
                .await
                .map(|v| Page {
                    items: v["data"]
                        .as_array()
                        .map(|data| {
                            data.iter()
                                .map(|d| SavedAlbum {
                                    added_at: d["attributes"]["dateAdded"]
                                        .as_str()
                                        .map(str::to_string),
                                    album: crate::mk::album(d),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    total: v["meta"]["total"].as_u64().unwrap_or(0) as u32,
                    limit: 100,
                    offset,
                    next: v["next"].as_str().map(str::to_string),
                })
                .map_err(err);
            Some(ApiResponse::SavedAlbums { offset, result })
        }
        ApiRequest::FollowedArtists { after } => {
            let result = mk
                .me_get("/library/artists", &[("limit", "100")])
                .await
                .map(|v| CursorPage {
                    items: v["data"]
                        .as_array()
                        .map(|data| data.iter().map(crate::mk::artist).collect())
                        .unwrap_or_default(),
                    total: v["meta"]["total"].as_u64().map(|n| n as u32),
                    next: v["next"].as_str().map(str::to_string),
                    cursors: None,
                })
                .map_err(err);
            Some(ApiResponse::FollowedArtists { after, result })
        }
        ApiRequest::TopTracks {
            offset,
            full,
            generation,
        } => {
            let result = mk.top_tracks(offset).await.map_err(err);
            Some(ApiResponse::TopTracks {
                offset,
                full,
                generation,
                result,
            })
        }
        ApiRequest::TopArtists { generation } => {
            let result = mk.top_artists().await.map_err(err);
            Some(ApiResponse::TopArtists { generation, result })
        }
        ApiRequest::Recommendations { generation, .. } => {
            let result = mk.recommendations().await.map_err(err);
            Some(ApiResponse::Recommendations { generation, result })
        }
        ApiRequest::CreatePlaylist {
            name, description, ..
        } => {
            let result = mk.playlist_create(&name, &description).await.map_err(err);
            Some(ApiResponse::PlaylistCreated(result))
        }
        ApiRequest::AddToPlaylist {
            playlist_id,
            playlist_name,
            uris,
            ..
        } => {
            let result = mk
                .playlist_add(&playlist_id, &uris)
                .await
                .map(|_| None)
                .map_err(err);
            Some(ApiResponse::PlaylistItemsChanged {
                id: playlist_id,
                message: format!("Added to {playlist_name}"),
                result,
            })
        }
        ApiRequest::RemoveFromPlaylist {
            playlist_id, uris, ..
        } => {
            let result = mk
                .playlist_remove(&playlist_id, &uris)
                .await
                .map(|_| None)
                .map_err(err);
            Some(ApiResponse::PlaylistItemsChanged {
                id: playlist_id,
                message: "Removed from playlist".to_string(),
                result,
            })
        }
        ApiRequest::CheckPlaylistDuplicates {
            playlist_id,
            playlist_name,
            items,
            position,
        } => {
            let uris: Vec<String> = items.iter().map(|i| i.uri().to_string()).collect();
            let result = mk
                .playlist_duplicates(&playlist_id, &uris)
                .await
                .map_err(err);
            Some(ApiResponse::PlaylistDuplicatesChecked {
                playlist_id,
                playlist_name,
                items,
                position,
                result,
            })
        }
        ApiRequest::FollowPlaylist { id, follow } => {
            let uris = vec![format!("melba:playlist:{id}")];
            let result = if follow {
                mk.save(&uris).await
            } else {
                mk.unsave(&uris).await
            }
            .map_err(err);
            Some(ApiResponse::PlaylistFollowChanged {
                id,
                followed: follow,
                result,
            })
        }
        // Apple Music's API rejects playlist rename/reorder/cover for
        // web-origin tokens (405/404 probed live) — answer honestly instead
        // of silently pretending success.
        ApiRequest::UpdatePlaylist { id, .. } => Some(ApiResponse::PlaylistUpdated {
            result: Err(ApiError::Network(
                "Apple Music doesn't allow editing playlist details".into(),
            )),
            id,
        }),
        ApiRequest::ReorderPlaylist { playlist_id, .. } => {
            Some(ApiResponse::PlaylistItemsChanged {
                id: playlist_id,
                message: String::new(),
                result: Err(ApiError::Network(
                    "Apple Music doesn't allow reordering playlist tracks".into(),
                )),
            })
        }
        ApiRequest::UploadPlaylistCover {
            id,
            request,
            previous_urls,
            cover,
        } => Some(ApiResponse::PlaylistCoverUploaded {
            result: Err(ApiError::Network(
                "Apple Music doesn't allow playlist covers".into(),
            )),
            id,
            request,
            previous_urls,
            cover,
        }),
        _ => None,
    }
}

/// The name, avatar and subscription state of the signed-in Apple account,
/// taken from the account data Apple's own client keeps for the managed
/// wrapper. Read-only, size-bounded, and limited to the fields the interface
/// shows: the display name, the profile image, whether the account is
/// subscribed, and its account id.
#[derive(Clone, Debug, Default, PartialEq)]
struct LocalAccount {
    id: Option<String>,
    name: Option<String>,
    images: Vec<Image>,
    subscribed: Option<bool>,
}

/// The wrapper's data files, under the user's data directory.
fn wrapper_account_files() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("melba/wrapper/rootfs/data/data/com.apple.android.music/files"))
}

/// Reads one bounded, regular, non-symlink JSON file.
fn read_account_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 1 << 20 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn account_name(account: &serde_json::Value) -> Option<String> {
    let first = account["firstName"].as_str().unwrap_or_default().trim();
    let last = account["lastName"].as_str().unwrap_or_default().trim();
    let name = format!("{first} {last}").trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// `AccountTable.json` → the active account's id, name and subscription state.
fn account_table(v: &serde_json::Value) -> LocalAccount {
    let accounts = v["accounts"].as_array().cloned().unwrap_or_default();
    let active = accounts
        .iter()
        .find(|account| account["active"].as_bool() == Some(true))
        .or_else(|| {
            let id = v["activeAccount"].as_str();
            accounts
                .iter()
                .find(|account| id.is_some_and(|id| account["id"].as_str() == Some(id)))
        })
        .or_else(|| accounts.first());
    let Some(account) = active else {
        return LocalAccount::default();
    };
    LocalAccount {
        // Apple stores the DSID as a number and the account id as a string.
        id: account["dsid"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| account["dsid"].as_i64().map(|dsid| dsid.to_string()))
            .or_else(|| account["id"].as_str().map(str::to_owned)),
        name: account_name(account),
        images: Vec::new(),
        // Only a boolean is trusted: the observed value is a number whose
        // meaning is unverified, and a wrong "not subscribed" label would be
        // worse than the placeholder Apple Music answered with.
        subscribed: account["imSubscribed"].as_bool(),
    }
}

/// `UserProfileTable.json` → the profile's own name and image.
fn profile_table(v: &serde_json::Value) -> LocalAccount {
    let Some(profile) = v["userProfiles"].as_array().and_then(|list| list.first()) else {
        return LocalAccount::default();
    };
    let name = profile["name"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let image = &profile["profileImage"];
    let width = image["width"]
        .as_u64()
        .and_then(|width| u32::try_from(width).ok());
    let height = image["height"]
        .as_u64()
        .and_then(|height| u32::try_from(height).ok());
    let url = image["url"].as_str().unwrap_or_default().trim();
    // Apple's image urls carry {w}x{h} placeholders; a dense rendition is
    // asked for once and the image loader scales down.
    let url = crate::mk::artwork_url(url, width.unwrap_or(512), height.unwrap_or(512));
    let images = (!url.is_empty())
        .then_some(Image { url, width, height })
        .into_iter()
        .collect();
    LocalAccount {
        id: None,
        name,
        images,
        subscribed: None,
    }
}

fn wrapper_account() -> LocalAccount {
    let Some(files) = wrapper_account_files() else {
        return LocalAccount::default();
    };
    let table = read_account_json(&files.join("AccountTable.json"))
        .map(|v| account_table(&v))
        .unwrap_or_default();
    let profile = read_account_json(&files.join("UserProfileTable.json"))
        .map(|v| profile_table(&v))
        .unwrap_or_default();
    LocalAccount {
        id: table.id,
        // The profile's own display name wins over the account's full name.
        name: profile.name.or(table.name),
        images: profile.images,
        subscribed: table.subscribed,
    }
}

/// /v1/me — the signed-in social profile. Apple reports
/// `{id, type: social-profile, attributes:{name}}` and, for an account with a
/// social profile, its artwork; the account data Apple's own client stored for
/// the managed wrapper fills in the name, avatar and subscription state that
/// `/v1/me` omits. A missing name leaves the UI on the account id rather than a
/// made-up label.
async fn profile(mk: &MusicKit) -> Option<User> {
    let remote = mk.me_get("", &[]).await.ok().and_then(|v| profile_user(&v));
    merge_profile(remote, wrapper_account())
}

/// `/v1/me`'s answer with the local account data filling its gaps. Apple only
/// shares a social profile's name, and never its avatar, so the stored account
/// data supplies the name, the profile image and the real subscription state;
/// either source alone is enough to show the account.
fn merge_profile(remote: Option<User>, local: LocalAccount) -> Option<User> {
    let mut user = remote.or_else(|| {
        let id = local.id.clone()?;
        Some(User {
            uri: Some(format!("melba:user:{id}")),
            id,
            ..User::default()
        })
    })?;
    if user.display_name.is_none() {
        user.display_name = local.name;
    }
    if user.images.is_empty() {
        user.images = local.images;
    }
    // A confirmed subscription state beats the placeholder: an unsubscribed
    // account must not be labelled "Apple Music".
    if let Some(subscribed) = local.subscribed {
        user.product = Some(if subscribed { "premium" } else { "free" }.to_owned());
    }
    Some(user)
}

/// `/v1/me` data → the app's user: real account id, name only when Apple
/// shares a non-empty one.
fn profile_user(v: &serde_json::Value) -> Option<User> {
    let profile = v["data"].as_array()?.first()?;
    let id = profile["id"].as_str()?;
    Some(User {
        id: id.into(),
        display_name: profile["attributes"]["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned),
        product: Some("premium".into()),
        uri: Some(format!("melba:user:{id}")),
        ..User::default()
    })
}

/// /v1/me/recent/played/tracks → the Recents row.
async fn recent(
    mk: &MusicKit,
    limit: u32,
) -> crate::backend::ApiResult<CursorPage<crate::api::models::PlayHistory>> {
    let v = mk
        .me_get(
            "/recent/played/tracks",
            &[("limit", &limit.min(25).to_string())],
        )
        .await
        .map_err(ApiError::Network)?;
    Ok(CursorPage {
        items: v["data"]
            .as_array()
            .map(|data| {
                data.iter()
                    .map(|d| crate::api::models::PlayHistory {
                        track: crate::mk::song(d),
                        played_at: d["attributes"]["lastPlayedDate"]
                            .as_str()
                            .map(str::to_string),
                        context: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        total: None,
        next: v["next"].as_str().map(str::to_string),
        cursors: None,
    })
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Shutdown => "Shutdown",
        Command::Player(_) => "Player",
        Command::Api(_) => "Api",
        Command::SignIn { .. } => "SignIn",
        Command::AuthStart { .. } => "AuthStart",
        Command::SignOut => "SignOut",
        Command::Reconnect => "Reconnect",
        _ => "other",
    }
}

fn event_name(event: &Event) -> &'static str {
    match event {
        Event::Auth(_) | Event::AuthAttempt { .. } => "Auth",
        Event::Playback(_) => "Playback",
        Event::Local(_) => "Local",
        Event::Api(_) => "Api",
        _ => "other",
    }
}

/// Model URIs carry `melba:track:<id>` (legacy saved state may still say
/// `spotify:track:`); the melbad daemon wants `stream:<id>` (full
/// length), `preview:<id>`, or `file://…`. Anything already
/// melbad-shaped passes through.
/// Playback and views must use the same API namespace, song parser and paging.
/// In particular, p.* library playlists are not catalog playlists, and their
/// i.* song ids must be mapped through playParams.catalogId by mk::song.
async fn resolve_context<F, Fut>(context: &str, mut fetch: F) -> Result<Vec<String>, String>
where
    F: FnMut(ApiRequest) -> Fut,
    Fut: std::future::Future<Output = Option<ApiResponse>>,
{
    let parts: Vec<_> = context.split(':').collect();
    let [scheme, kind, id] = parts.as_slice() else {
        return Err("unsupported context shape".into());
    };
    if !matches!(*scheme, "melba" | "spotify") {
        return Err("unsupported context scheme".into());
    }
    if *kind == "track" {
        return Ok(vec![context.to_owned()]);
    }
    let mut offset = 0u32;
    let mut uris = Vec::new();
    let mut next_pages = std::collections::HashSet::new();
    loop {
        let request = match *kind {
            "playlist" => ApiRequest::PlaylistItems {
                id: (*id).into(),
                offset,
                generation: 0,
            },
            "album" => ApiRequest::AlbumTracks {
                id: (*id).into(),
                offset,
                generation: 0,
            },
            "artist" => ApiRequest::ArtistTopTracks { id: (*id).into() },
            _ => return Err("unsupported context kind".into()),
        };
        let response = fetch(request)
            .await
            .ok_or("context request returned no response")?;
        let (items, count, next): (Vec<String>, usize, Option<String>) = match response {
            ApiResponse::PlaylistItems { result, .. } => {
                let page = result.map_err(|_| "playlist context request failed")?;
                let count = page.items.len();
                (
                    page.items
                        .into_iter()
                        .filter_map(|i| i.item.map(|i| i.uri().to_owned()))
                        .collect(),
                    count,
                    page.next,
                )
            }
            ApiResponse::AlbumTracks { result, .. } => {
                let page = result.map_err(|_| "album context request failed")?;
                let count = page.items.len();
                (
                    page.items.into_iter().map(|t| t.uri).collect(),
                    count,
                    page.next,
                )
            }
            ApiResponse::ArtistTopTracks { result, .. } => {
                let tracks = result.map_err(|_| "artist context request failed")?;
                let count = tracks.len();
                (tracks.into_iter().map(|t| t.uri).collect(), count, None)
            }
            _ => return Err("context request returned an unexpected response".into()),
        };
        uris.extend(items);
        let Some(next) = next else {
            break;
        };
        if count == 0 || !next_pages.insert(next) {
            return Err("context pagination made no progress".into());
        }
        offset = offset
            .checked_add(u32::try_from(count).map_err(|_| "context page too large")?)
            .ok_or("context offset overflow")?;
    }
    if uris.is_empty() {
        Err("context contains no playable items".into())
    } else {
        Ok(uris)
    }
}

fn daemon_uri(uri: &str) -> String {
    if let Some(id) = uri
        .strip_prefix("melba:track:")
        .or_else(|| uri.strip_prefix("spotify:track:"))
    {
        return format!("stream:{id}");
    }
    uri.to_string()
}

/// The inverse of [`daemon_uri`]: a `stream:<id>` the daemon echoes back
/// in its metadata becomes `melba:track:<id>` again so now-playing rows
/// match the tracks the app asked for. `preview:` and `file://` stay
/// daemon-shaped; they name different content.
fn model_uri(uri: &str) -> String {
    uri.strip_prefix("stream:")
        .map(|id| format!("melba:track:{id}"))
        .unwrap_or_else(|| uri.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        LocalAccount, account_table, daemon_uri, merge_profile, model_uri, profile_table,
        profile_user, sign_in_command,
    };
    use crate::api::models::Image;

    #[test]
    fn a_daemon_that_exits_is_reaped_and_restarted_when_nothing_serves_mpris() {
        use super::DaemonAction as A;
        use super::{DAEMON_RESTART_COOLDOWN as cooldown, DAEMON_RESTART_LIMIT as limit};
        use super::DaemonWorker;
        // No owned child, or one still running: nothing to do.
        assert_eq!(DaemonWorker::daemon_action(false, false, false, None, 0), A::Idle);
        assert_eq!(DaemonWorker::daemon_action(true, false, false, None, 0), A::Idle);
        // Exited, and the bus has no MPRIS owner: restart it.
        assert_eq!(
            DaemonWorker::daemon_action(true, true, false, None, 0),
            A::Restart
        );
        // Exited, but something else answers: reap ours, leave theirs alone.
        assert_eq!(
            DaemonWorker::daemon_action(true, true, true, None, 0),
            A::Reap
        );
        // A crash loop must not spin: the cooldown and the limit both stop it.
        assert_eq!(
            DaemonWorker::daemon_action(true, true, false, Some(cooldown / 2), 0),
            A::GiveUp
        );
        assert_eq!(
            DaemonWorker::daemon_action(true, true, false, Some(cooldown * 2), 0),
            A::Restart
        );
        assert_eq!(
            DaemonWorker::daemon_action(true, true, false, Some(cooldown * 2), limit),
            A::GiveUp
        );
    }

    #[test]
    fn the_stored_account_supplies_the_name_avatar_and_subscription() {
        let table = serde_json::json!({
            "accounts": [
                { "active": false, "dsid": "0", "firstName": "Other", "imSubscribed": false },
                { "active": true, "dsid": "12345", "firstName": "Yuu", "lastName": "K",
                  "imSubscribed": true }
            ]
        });
        let profile = serde_json::json!({
            "userProfiles": [
                { "name": "Yuu", "profileImage": { "width": 500, "height": 500,
                                                   "url": "https://img/500x500.jpg" } }
            ]
        });
        let account = account_table(&table);
        assert_eq!(account.id.as_deref(), Some("12345"));
        assert_eq!(account.name.as_deref(), Some("Yuu K"));
        assert_eq!(account.subscribed, Some(true));
        let local = profile_table(&profile);
        assert_eq!(local.name.as_deref(), Some("Yuu"));
        assert_eq!(local.images.len(), 1);
        assert_eq!(local.images[0].url, "https://img/500x500.jpg");
        assert_eq!(local.images[0].width, Some(500));
    }

    #[test]
    fn profile_image_template_is_a_fetchable_rendition() {
        // The native account's template includes crop and format as well as
        // dimensions. Leaving either placeholder gives HTTP 400, not an image.
        let profile = profile_table(&serde_json::json!({
            "userProfiles": [{"profileImage": {
                "width": 500, "height": 500,
                "url": "https://img/{w}x{h}{c}.{f}"
            }}]
        }));
        assert_eq!(profile.images[0].url, "https://img/500x500bb.jpg");
    }

    #[test]
    fn the_profile_merge_fills_what_apple_omits_and_keeps_what_it_sends() {
        let local = LocalAccount {
            id: Some("12345".into()),
            name: Some("Yuu".into()),
            images: vec![Image {
                url: "https://img/500x500.jpg".into(),
                width: Some(500),
                height: Some(500),
            }],
            subscribed: Some(false),
        };
        // Apple answered with a name and no artwork; the stored image fills in.
        let remote = profile_user(&serde_json::json!({
            "data": [ { "id": "social-1", "attributes": { "name": "Dee" } } ]
        }));
        let merged = merge_profile(remote, local.clone()).expect("merged");
        assert_eq!(merged.id, "social-1");
        assert_eq!(merged.display_name.as_deref(), Some("Dee"));
        assert_eq!(merged.images.len(), 1);
        // An unsubscribed account is never labelled "Apple Music".
        assert_eq!(merged.product.as_deref(), Some("free"));

        // Apple said nothing: the stored account still shows.
        let merged = merge_profile(None, local.clone()).expect("local account");
        assert_eq!(merged.id, "12345");
        assert_eq!(merged.display_name.as_deref(), Some("Yuu"));
        assert_eq!(merged.uri.as_deref(), Some("melba:user:12345"));
        assert_eq!(merged.product.as_deref(), Some("free"));

        // Neither source: no account to show.
        assert!(merge_profile(None, LocalAccount::default()).is_none());
    }

    #[test]
    fn an_empty_or_missing_profile_table_contributes_nothing() {
        assert_eq!(
            account_table(&serde_json::json!({ "accounts": [] })),
            LocalAccount::default()
        );
        let profile = profile_table(&serde_json::json!({
            "userProfiles": [ { "name": "   ", "profileImage": {} } ]
        }));
        assert!(profile.name.is_none());
        assert!(profile.images.is_empty());
        assert!(profile_table(&serde_json::json!({})).images.is_empty());
    }

    #[test]
    fn resolved_context_keeps_the_selected_track_and_rejects_stale_loads() {
        const CHILD: &str = "MELBA_CONTEXT_PRIVATE_BUS";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new("dbus-run-session")
                .arg("--")
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", "daemon_worker::tests::resolved_context_keeps_the_selected_track_and_rejects_stale_loads", "--nocapture"])
                .env(CHILD, "1")
                .env("MELBA_CONTEXT_PARENT_BUS", std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default())
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
            return;
        }
        assert_ne!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            std::env::var("MELBA_CONTEXT_PARENT_BUS")
                .expect("isolated child needs its parent bus identity")
        );
        use dbus::channel::{MatchingReceiver, Sender as _};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;
        let (opened, calls) = std::sync::mpsc::channel();
        let (ready, started) = std::sync::mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let server = std::thread::spawn(move || {
            let conn = dbus::blocking::Connection::new_session().unwrap();
            assert_eq!(
                conn.request_name(super::BUS, false, false, true).unwrap(),
                dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply::PrimaryOwner
            );
            // Model the daemon's queue operations rather than blessing only
            // OpenUri: that missed old contexts accumulating and row order loss.
            let mut queue = (vec!["stream:old-context".to_string()], 0u32, 0u32, false);
            conn.start_receive(
                dbus::message::MatchRule::new_method_call(),
                Box::new(move |msg, conn| {
                    match msg.member().map(|m| m.to_string()).as_deref() {
                        Some("OpenUri") => {
                            queue.0.push(msg.read1::<String>().unwrap());
                            queue.1 = queue.0.len() as u32 - 1;
                        }
                        Some("MelbaEnqueue") => queue.0.extend(msg.read1::<Vec<String>>().unwrap()),
                        Some("Play") => {
                            queue.3 = true;
                            opened.send(queue.clone()).unwrap();
                        }
                        Some("MelbaLoadQueue") => {
                            queue = msg.read4::<Vec<String>, u32, u32, bool>().unwrap();
                            opened.send(queue.clone()).unwrap();
                        }
                        _ => {}
                    }
                    conn.send(msg.method_return()).unwrap();
                    true
                }),
            );
            ready.send(()).unwrap();
            while !stop.load(Ordering::Acquire) {
                conn.process(Duration::from_millis(20)).unwrap();
            }
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        let (events, _) = std::sync::mpsc::channel();
        let mut worker = super::DaemonWorker::new(
            events,
            crate::backend::Waker::default(),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            crate::settings::ProxyConfig::Off,
        );
        let tracks = vec![
            "melba:track:101".to_string(),
            "melba:track:202".into(),
            "melba:track:303".into(),
        ];
        worker.handle_player(crate::player::PlayerCommand::Load(
            crate::player::LoadSpec {
                uris: vec![tracks[0].clone()],
                play: true,
                ..Default::default()
            },
        ));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            (vec!["stream:101".into()], 0, 0, true)
        );
        for selection in [
            crate::player::LoadSpec {
                offset_uri: Some(tracks[1].clone()),
                ..Default::default()
            },
            crate::player::LoadSpec {
                offset_index: Some(1),
                ..Default::default()
            },
            // App::local_load emits the legacy wire scheme, whereas the
            // MusicKit view parser returns canonical melba: URIs.
            crate::player::LoadSpec {
                offset_uri: Some("spotify:track:202".into()),
                ..Default::default()
            },
        ] {
            worker.context_resolved(
                worker.load_epoch,
                crate::player::LoadSpec {
                    context_uri: Some("melba:album:fixture".into()),
                    position_ms: 640,
                    play: true,
                    ..selection
                },
                tracks.clone(),
            );
            assert_eq!(
                calls.recv_timeout(Duration::from_secs(2)).unwrap(),
                (
                    vec![
                        "stream:101".into(),
                        "stream:202".into(),
                        "stream:303".into()
                    ],
                    1,
                    640,
                    true
                )
            );
        }
        // A previously requested album resolves after a newer direct play.
        let old = worker.load_epoch;
        worker.handle_player(crate::player::PlayerCommand::Load(
            crate::player::LoadSpec {
                uris: vec![tracks[2].clone()],
                play: true,
                ..Default::default()
            },
        ));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            (vec!["stream:303".into()], 0, 0, true)
        );
        worker.context_resolved(old, crate::player::LoadSpec::default(), tracks);
        assert!(
            calls.try_recv().is_err(),
            "late context must not undo the newer selection"
        );
        worker.handle_player(crate::player::PlayerCommand::Load(
            crate::player::LoadSpec {
                uris: vec![
                    "melba:track:101".into(),
                    "melba:track:202".into(),
                    "melba:track:101".into(),
                    "melba:track:303".into(),
                ],
                offset_uri: Some("spotify:track:101".into()),
                offset_index: Some(2),
                position_ms: 150,
                play: false,
                ..Default::default()
            },
        ));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap(),
            (
                vec![
                    "stream:101".into(),
                    "stream:202".into(),
                    "stream:101".into(),
                    "stream:303".into()
                ],
                2,
                150,
                false
            ),
            "an explicit row selects that occurrence, not the first equal URI"
        );
        done.store(true, Ordering::Release);
        server.join().unwrap();
    }

    #[test]
    fn library_context_uses_the_view_loader_and_catalog_song_ids_across_pages() {
        let mut requests = Vec::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let uris = runtime.block_on(super::resolve_context("spotify:playlist:p.fixture", |request| {
            let crate::backend::ApiRequest::PlaylistItems { id, offset, .. } = request else { panic!("playback must use the same playlist loader as the view"); };
            requests.push((id.clone(), offset));
            let ids: &[(&str, &str)] = match offset { 0 => &[("i.library-a", "101"), ("i.library-b", "202")], 2 => &[("i.library-c", "303")], _ => panic!("unexpected page") };
            let items = ids.iter().map(|(library, catalog)| crate::api::models::PlaylistItem {
                item: Some(crate::api::models::PlayableItem::Track(crate::mk::song(&serde_json::json!({
                    "id": library, "type": "library-songs", "attributes": { "playParams": {"catalogId": catalog} }
                })))), ..Default::default()
            }).collect();
            std::future::ready(Some(crate::backend::ApiResponse::PlaylistItems {
                id, offset, generation: 0,
                result: Ok(crate::api::models::Page { items, offset, total: 3, limit: 2,
                    next: (offset == 0).then(|| "/v1/me/library/playlists/p.fixture/tracks?offset=2".into()) })
            }))
        })).unwrap();
        assert_eq!(
            requests,
            vec![("p.fixture".into(), 0), ("p.fixture".into(), 2)]
        );
        assert_eq!(
            uris,
            ["melba:track:101", "melba:track:202", "melba:track:303"]
        );
    }

    #[test]
    fn saved_tracks_reply_preserves_the_request_account_even_on_failure() {
        const CHILD: &str = "MELBA_SAVED_ACCOUNT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root =
                std::env::temp_dir().join(format!("melba-saved-account-{}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let name = std::thread::current().name().unwrap().to_owned();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"])
                .env(CHILD, "1")
                .env("HOME", &root)
                .env("XDG_CONFIG_HOME", &root)
                .env("XDG_CACHE_HOME", &root)
                .output()
                .unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("test result: ok. 1 passed; 0 failed;")
            );
            return;
        }
        assert!(crate::mk::MusicKit::user_token().is_none());
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let mk = crate::mk::MusicKit::new(http);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let reply = runtime
            .block_on(super::catalog_answer(
                &mk,
                crate::backend::ApiRequest::SavedTracks {
                    offset: 2,
                    generation: 7,
                    account_id: Some("fixture-account".into()),
                },
            ))
            .unwrap();
        let crate::backend::ApiResponse::SavedTracks {
            offset,
            generation,
            account_id,
            result,
        } = reply
        else {
            panic!("wrong response");
        };
        assert_eq!(offset, 2);
        assert_eq!(generation, 7);
        assert_eq!(account_id.as_deref(), Some("fixture-account"));
        assert!(
            result.is_err(),
            "missing fixture token must fail before network access"
        );
    }

    #[test]
    fn context_pagination_refuses_an_empty_continuation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut calls = 0;
        let result = runtime.block_on(super::resolve_context("melba:playlist:p.fixture", |_| {
            calls += 1;
            std::future::ready(Some(crate::backend::ApiResponse::PlaylistItems {
                id: "p.fixture".into(),
                offset: 0,
                generation: 0,
                result: Ok(crate::api::models::Page {
                    next: Some("/next".into()),
                    ..Default::default()
                }),
            }))
        }));
        assert!(result.unwrap_err().contains("no progress"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn disabled_restore_never_starts_native_authentication() {
        let (events, received) = std::sync::mpsc::channel();
        let mut worker = super::DaemonWorker::new(
            events,
            crate::backend::Waker::default(),
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            crate::settings::ProxyConfig::Off,
        );
        // Deliberately leave the child-spawn channel absent: even a broken
        // startup guard cannot execute a real helper or read an account.
        worker.restore_session(false);
        assert!(received.try_recv().is_err());
        worker.restore_session(true);
        assert!(matches!(
            received.try_recv().unwrap(),
            crate::backend::Event::AuthAttempt {
                status: crate::backend::AuthStatus::Connecting,
                ..
            }
        ));
    }

    #[test]
    fn sign_in_uses_coinstalled_daemon_before_path_and_keeps_path_fallback() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "melba-sign-in-command-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(directory.join("app")).unwrap();
        std::fs::create_dir_all(directory.join("path")).unwrap();
        let marker = directory.join("invocation");
        for (folder, label) in [("app", "sibling"), ("path", "fallback")] {
            let binary = directory.join(folder).join("melbad");
            std::fs::write(
                &binary,
                format!("#!/bin/sh\nprintf '{label}:%s' \"$*\" > \"$MARKER\"\n"),
            )
            .unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let executable = directory.join("app/melba");
        let invoke = |executable| {
            let status = sign_in_command(executable)
                .env("PATH", directory.join("path"))
                .env("MARKER", &marker)
                .status()
                .unwrap();
            (status.success(), std::fs::read_to_string(&marker).unwrap())
        };
        let coinstalled = invoke(Some(executable.clone()));
        std::fs::remove_file(directory.join("app/melbad")).unwrap();
        let separate_install = invoke(Some(executable));
        let unavailable_executable = invoke(None);
        std::fs::remove_dir_all(directory).unwrap();

        assert_eq!(coinstalled, (true, "sibling:auth gui".into()));
        assert_eq!(separate_install, (true, "fallback:auth gui".into()));
        assert_eq!(unavailable_executable, separate_install);
    }

    #[test]
    fn track_uris_map_to_stream() {
        assert_eq!(daemon_uri("melba:track:1588049004"), "stream:1588049004");
        // Legacy spotify: scheme saved state maps the same way.
        assert_eq!(daemon_uri("spotify:track:1588049004"), "stream:1588049004");
    }

    #[test]
    fn stream_uris_map_back_to_model_tracks() {
        assert_eq!(model_uri("stream:1588049004"), "melba:track:1588049004");
        assert_eq!(model_uri(&daemon_uri("melba:track:1")), "melba:track:1");
        // Previews and files are not full tracks; they stay as they are.
        assert_eq!(model_uri("preview:2"), "preview:2");
        assert_eq!(model_uri("file:///a/b.flac"), "file:///a/b.flac");
    }

    #[test]
    fn profile_reads_the_social_profile_id_and_name() {
        let user = profile_user(&serde_json::json!({
            "data": [{"id": "u-42", "type": "social-profile",
                      "attributes": {"name": "Yuu"}}]
        }))
        .expect("profile");
        assert_eq!(user.id, "u-42");
        assert_eq!(user.display_name.as_deref(), Some("Yuu"));
        assert_eq!(user.name(), "Yuu");
        // A blank or missing name falls back to the account id.
        for v in [
            serde_json::json!({"data": [{"id": "u-43", "attributes": {"name": "  "}}]}),
            serde_json::json!({"data": [{"id": "u-43", "attributes": {}}]}),
        ] {
            let user = profile_user(&v).expect("profile");
            assert_eq!(user.name(), "u-43");
        }
        // Anything less than an id is not a profile.
        for v in [
            serde_json::json!({"data": []}),
            serde_json::json!({"data": [{"attributes": {"name": "Yuu"}}]}),
            serde_json::json!({}),
        ] {
            assert!(profile_user(&v).is_none());
        }
    }

    #[test]
    fn daemon_and_file_uris_pass_through() {
        assert_eq!(daemon_uri("stream:1"), "stream:1");
        assert_eq!(daemon_uri("preview:2"), "preview:2");
        assert_eq!(daemon_uri("file:///a/b.flac"), "file:///a/b.flac");
    }

    #[test]
    fn non_track_context_is_not_rewritten() {
        assert_eq!(
            daemon_uri("melba:album:1588048900"),
            "melba:album:1588048900"
        );
    }
}

#[cfg(test)]
#[path = "daemon_worker/desktop_tests.rs"]
mod desktop_tests;
