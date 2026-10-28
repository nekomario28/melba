//! Bridge between the UI thread and asynchronous work.
//!
//! egui runs on the main thread and must never block. A dedicated tokio
//! runtime hosts the daemon worker, service requests, sign-in, and
//! artwork fetches; the two sides talk through channels. Every event wakes
//! the interface with `request_repaint`, so the app stays event-driven and
//! idle when nothing is happening.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::api::ApiError;
use crate::api::models::*;
use crate::http::Http;
use crate::images::ArtLoader;
use crate::model::PlaylistCache;
use crate::paths::AppDirs;
use crate::player::{LocalState, PlayerCommand};
use crate::settings::ProxyConfig;

pub type ApiResult<T> = Result<T, ApiError>;

pub const PLAYLIST_PAGE_SIZE: u32 = 50;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthStatus {
    Starting,
    SignedOut,
    WaitingForBrowser {
        url: String,
    },
    Prompt {
        attempt: u64,
        prompt: melba_auth::Prompt,
    },
    Connecting,
    Connected {
        username: String,
    },
    Failed(String),
}

/// Which of the two readers of the recently-played endpoint an answer
/// belongs to: the shelf on Home, or the Recents tab in the queue panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentsFor {
    Home,
    Panel,
}

#[derive(Clone, Debug)]
pub enum ApiRequest {
    Me,
    Devices,
    AudioDevices,
    SetAudioDevice {
        device: String,
    },
    PlaybackState {
        seq: u64,
    },
    Queue {
        seq: u64,
    },
    RecentlyPlayed {
        /// Request owner. Home and Recents use separate generation counters,
        /// so generation alone cannot route the response.
        who: RecentsFor,
        generation: u64,
        before: Option<String>,
        limit: u32,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
    },
    TopArtists {
        generation: u64,
    },
    Recommendations {
        seed_tracks: Vec<String>,
        seed_artists: Vec<String>,
        generation: u64,
    },
    MadeForYou {
        generation: u64,
        force: bool,
    },
    MyPlaylists {
        offset: u32,
        generation: u64,
    },
    Playlist {
        id: String,
        generation: u64,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
    },
    CreatePlaylist {
        name: String,
        public: bool,
        description: String,
    },
    UploadPlaylistCover {
        id: String,
        request: u64,
        previous_urls: Vec<String>,
        cover: crate::playlist_cover::Cover,
    },
    UpdatePlaylist {
        id: String,
        name: Option<String>,
        description: Option<String>,
        public: Option<bool>,
    },
    CheckPlaylistDuplicates {
        playlist_id: String,
        playlist_name: String,
        items: Vec<PlayableItem>,
        position: Option<u32>,
    },
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        uris: Vec<String>,
        position: Option<u32>,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        uris: Vec<String>,
        snapshot_id: Option<String>,
    },
    ReorderPlaylist {
        playlist_id: String,
        range_start: u32,
        insert_before: u32,
        snapshot_id: Option<String>,
    },
    FollowPlaylist {
        id: String,
        follow: bool,
    },
    SavedTracks {
        offset: u32,
        generation: u64,
        account_id: Option<String>,
    },
    SavedAlbums {
        offset: u32,
    },
    FollowedArtists {
        after: Option<String>,
    },
    SetSaved {
        uris: Vec<String>,
        saved: bool,
    },
    Contains {
        uris: Vec<String>,
    },
    Search {
        query: String,
        serial: u64,
    },
    /// Search for one kind of thing and report the best match, to play it:
    /// how `melba play-search` plays a song by its name.
    PlaySearch {
        kind: String,
        query: String,
        serial: u64,
    },
    Artist {
        id: String,
    },
    ArtistTopTracks {
        id: String,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
    },
    RelatedArtists {
        id: String,
    },
    Album {
        id: String,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        generation: u64,
    },
    AlbumQueueTracks {
        id: String,
        offset: u32,
        request: u64,
    },
    Track {
        id: String,
    },
    AddToQueue {
        uri: String,
        device_id: Option<String>,
        label: String,
    },
    AddManyToQueue {
        request: u64,
        uris: Vec<String>,
    },
}

#[derive(Debug)]
pub enum ApiResponse {
    Me(ApiResult<User>),
    Devices(ApiResult<Vec<Device>>),
    AudioDevices(ApiResult<AudioOutputs>),
    AudioDeviceSet(ApiResult<()>),
    PlaybackState {
        seq: u64,
        result: ApiResult<Option<PlaybackState>>,
    },
    Queue {
        seq: u64,
        result: ApiResult<Queue>,
    },
    RecentlyPlayed {
        who: RecentsFor,
        generation: u64,
        limit: u32,
        result: ApiResult<CursorPage<PlayHistory>>,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    TopArtists {
        generation: u64,
        result: ApiResult<Vec<Artist>>,
    },
    Recommendations {
        generation: u64,
        result: ApiResult<Vec<Track>>,
    },
    MadeForYou {
        generation: u64,
        personalized: bool,
        result: ApiResult<Vec<Playlist>>,
    },
    MyPlaylists {
        offset: u32,
        generation: u64,
        result: ApiResult<Page<Playlist>>,
    },
    Playlist {
        id: String,
        generation: u64,
        result: ApiResult<Playlist>,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistCreated(ApiResult<Playlist>),
    PlaylistCoverUploaded {
        id: String,
        request: u64,
        previous_urls: Vec<String>,
        cover: crate::playlist_cover::Cover,
        result: ApiResult<()>,
    },
    PlaylistUpdated {
        id: String,
        result: ApiResult<()>,
    },
    PlaylistDuplicatesChecked {
        playlist_id: String,
        playlist_name: String,
        items: Vec<PlayableItem>,
        position: Option<u32>,
        result: ApiResult<Vec<String>>,
    },
    PlaylistItemsChanged {
        id: String,
        message: String,
        result: ApiResult<Option<String>>,
    },
    PlaylistFollowChanged {
        id: String,
        followed: bool,
        result: ApiResult<()>,
    },
    SavedTracks {
        offset: u32,
        generation: u64,
        account_id: Option<String>,
        result: ApiResult<Page<SavedTrack>>,
    },
    SavedAlbums {
        offset: u32,
        result: ApiResult<Page<SavedAlbum>>,
    },
    FollowedArtists {
        after: Option<String>,
        result: ApiResult<CursorPage<Artist>>,
    },
    SavedChanged {
        uris: Vec<String>,
        saved: bool,
        result: ApiResult<()>,
    },
    Contains {
        uris: Vec<String>,
        result: ApiResult<Vec<bool>>,
    },
    Search {
        query: String,
        serial: u64,
        result: ApiResult<SearchResults>,
    },
    /// The URI of the best match for a [`ApiRequest::PlaySearch`], or `None`
    /// when nothing matched.
    PlaySearch {
        query: String,
        serial: u64,
        result: ApiResult<Option<String>>,
    },
    Artist {
        id: String,
        result: ApiResult<Artist>,
    },
    ArtistTopTracks {
        id: String,
        result: ApiResult<Vec<Track>>,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
        result: ApiResult<Page<Album>>,
    },
    RelatedArtists {
        id: String,
        result: ApiResult<Vec<Artist>>,
    },
    Album {
        id: String,
        result: ApiResult<Album>,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    AlbumQueueTracks {
        offset: u32,
        request: u64,
        result: ApiResult<Page<Track>>,
    },
    Track {
        id: String,
        result: ApiResult<Track>,
    },
    QueueAdded {
        label: String,
        result: ApiResult<()>,
    },
    QueueBatchAdded {
        request: u64,
        added: usize,
        result: ApiResult<()>,
    },
}

pub enum PlaylistCacheRows {
    Replace(Vec<PlaylistItem>),
    Append {
        previous_rows: usize,
        previous_offset: u32,
        items: Vec<PlaylistItem>,
    },
}

/// Work for the daemon worker, sent over the command channel.
pub enum Command {
    OpenThemesFolder,
    CheckPlaylistCover {
        id: String,
        request: u64,
        cover: crate::playlist_cover::Cover,
        images: Vec<crate::api::models::Image>,
    },
    ChoosePlaylistCover {
        id: String,
        request: u64,
        selected:
            std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>,
    },
    /// Start (or restart) service sign-in.
    SignIn {
        request: u64,
        config: ProxyConfig,
    },
    /// Internal: generation assigned synchronously before enqueueing.
    AuthStart {
        attempt: u64,
        restore: bool,
    },
    AuthInput {
        attempt: u64,
        input: melba_auth::Input,
    },
    CancelSignIn,
    SignOut,
    Player(PlayerCommand),
    Api(ApiRequest),
    Accent {
        url: String,
    },
    Shutdown,
    /// OS reports network reachability changed.
    NetworkStatus(bool),
    /// Internal: the daemon asks for a reconnect.
    Reconnect,
    /// Ask GitHub whether a newer release exists. Manual checks report every
    /// outcome; the daily check only announces a new release.
    CheckForUpdates {
        manual: bool,
        source: crate::updates::Source,
    },
    InspectUpdate,
    /// Keep update HTTP requests on the network policy applied by the app.
    UpdateProxy(ProxyConfig),
    DownloadUpdate {
        release: crate::updates::Release,
        source: crate::updates::Source,
    },
    InstallUpdate {
        prepared: Box<crate::updates::Prepared>,
        arguments: Vec<String>,
    },
    /// The words of a track, from LRCLIB.
    Lyrics(Box<LyricsRequest>),
    /// Read a playlist's cached items from disk.
    LoadPlaylistCache {
        id: String,
        generation: u64,
    },
    /// Remember a playlist prefix on disk under its snapshot.
    StorePlaylistCache {
        id: String,
        generation: u64,
        snapshot: String,
        rows: PlaylistCacheRows,
        total: u32,
        next_offset: Option<u32>,
    },
    LoadLikedSongsCache {
        generation: u64,
    },
    StoreLikedSongsCache(crate::liked::Cache),
    /// Read the verified account's last playlist list and Home shelves.
    LoadAccountCaches {
        generation: u64,
    },
    StoreLibraryCache(crate::account_cache::Cache<Vec<Playlist>>),
    StoreHomeCache(crate::account_cache::Cache<crate::model::HomeSnapshot>),
    /// Resolve Spotify's radio seeded by `seed` through the streaming session.
    Radio {
        seed: String,
        generation: u64,
    },
}

pub struct LyricsRequest {
    /// The track the answer is for, so a stale one is ignored.
    pub uri: String,
    pub query: crate::lyrics::Query,
}

pub enum Event {
    AuthAttempt {
        attempt: u64,
        status: AuthStatus,
    },
    UpdateSupport(Result<crate::updates::Installation, String>),
    UpdateProgress {
        received: u64,
        total: u64,
    },
    UpdateDownloaded(Result<Box<crate::updates::Prepared>, String>),
    UpdateInstalling(Result<(), String>),
    PlaylistCoverChecked {
        id: String,
        request: u64,
        images: Vec<crate::api::models::Image>,
        result: Result<bool, String>,
    },
    PlaylistCoverChosen {
        id: String,
        request: u64,
        result: Result<Option<crate::playlist_cover::Cover>, String>,
    },
    Auth(AuthStatus),
    Playback(LocalPlayback),
    Local(Box<LocalState>),
    Api(Box<ApiResponse>),
    Accent {
        url: String,
        color: [u8; 3],
    },
    Error(String),
    /// GitHub answered an update check, or the request failed.
    UpdateChecked {
        manual: bool,
        result: Result<Option<crate::updates::Release>, String>,
    },
    /// Track lyrics, or `None` when unavailable.
    Lyrics {
        uri: String,
        result: Result<Option<crate::lyrics::Lyrics>, String>,
    },
    /// The result of reading a playlist cache for this load generation.
    PlaylistCache {
        account_id: String,
        id: String,
        generation: u64,
        cache: Option<PlaylistCache>,
    },
    PlaylistCacheStored {
        account_id: String,
        id: String,
        generation: u64,
        snapshot: String,
        success: bool,
    },
    /// The songs of the radio seeded by `seed`, for the request `generation`.
    Radio {
        seed: String,
        generation: u64,
        result: Result<Vec<crate::api::models::Track>, String>,
    },
    LikedSongsCache {
        account_id: String,
        generation: u64,
        cache: Option<crate::liked::Cache>,
    },
    /// OS reports network reachability changed.
    NetworkStatus(bool),
    AccountCaches {
        account_id: String,
        generation: u64,
        playlists: Option<Vec<Playlist>>,
        home: Option<crate::model::HomeSnapshot>,
    },
}

/// The state of playback on this computer, independent of the service sign-in.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalPlayback {
    /// Not authorized; local playback is unavailable but the app still works.
    Unavailable,
    /// The browser is open for the playback grant.
    Authorizing,
    /// Connecting to the daemon.
    Connecting,
    /// Network path is unavailable; reconnection suspended pending reachability.
    WaitingForNetwork,
    /// This computer is a ready Spotify Connect device.
    Ready {
        device_id: String,
    },
    Failed(String),
}

/// Background services (the runtime, MPRIS, the tray) outlive individual
/// windows: the window is destroyed when it closes to the tray and created
/// again on demand. They therefore hold this handle, which repaints
/// whichever window exists, instead of an `egui::Context`.
pub use fastframe_shell::Waker;

/// The interface's handle to the runtime.
pub struct Backend {
    auth_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    commands: mpsc::UnboundedSender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    art: ArtLoader,
    thread: Option<std::thread::JoinHandle<()>>,
    offline: bool,
    #[cfg(test)]
    network_statuses: std::sync::Mutex<Vec<bool>>,
    #[cfg(test)]
    playlist_item_requests: std::sync::Mutex<Vec<(String, u32, u64)>>,
    #[cfg(test)]
    #[cfg(test)]
    playlist_add_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    #[cfg(test)]
    #[cfg(test)]
    #[cfg(test)]
    queue_requests: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    sent_api: std::sync::Mutex<Vec<ApiRequest>>,
    #[cfg(test)]
    queued_tracks: std::sync::Mutex<Vec<String>>,
    #[cfg(all(test, target_os = "linux"))]
    player_loads: std::sync::Mutex<Vec<crate::player::LoadSpec>>,
    #[cfg(test)]
    #[cfg(test)]
    #[cfg(test)]
    #[cfg(test)]
    library_cache_stores: std::sync::Mutex<Vec<crate::account_cache::Cache<Vec<Playlist>>>>,
    #[cfg(test)]
    home_cache_stores:
        std::sync::Mutex<Vec<crate::account_cache::Cache<crate::model::HomeSnapshot>>>,
}

impl Backend {
    pub fn spawn(dirs: AppDirs, proxy: ProxyConfig, waker: Waker, restore_sign_in: bool) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let auth_epoch = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_epoch = std::sync::Arc::clone(&auth_epoch);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("melba-runtime")
            .enable_all()
            .build()
            .expect("unable to start the async runtime");
        let http = Http::from_proxy(&proxy).unwrap_or_else(|error| {
            let _ = event_tx.send(Event::Error(format!(
                "Network configuration failed: {error}"
            )));
            Http::unavailable(error)
        });
        let art = ArtLoader::new(http.clone(), runtime.handle().clone(), dirs.art_cache_dir());
        // The daemon must resolve the same directories as this app.
        let daemon_dirs = crate::daemon_worker::DaemonDirs {
            config: dirs.config.clone(),
            state: dirs.state.clone(),
            cache: dirs.cache.clone(),
        };

        let thread = std::thread::Builder::new()
            .name("melba-backend".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    #[cfg(target_os = "linux")]
                    crate::daemon_worker::DaemonWorker::new(
                        event_tx,
                        waker,
                        worker_epoch,
                        proxy,
                        daemon_dirs,
                    )
                    // Demo/headless sessions explicitly disable bootstrap:
                    // they must never start or stop the user's daemon.
                    .run(command_rx, restore_sign_in, restore_sign_in)
                    .await;
                    #[cfg(not(target_os = "linux"))]
                    let _ = (event_tx, waker, command_rx, worker_epoch, restore_sign_in);
                });
                // Give the worker's own threads a moment to release the audio device.
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .expect("unable to start the backend thread");

        Self {
            auth_epoch,
            commands: command_tx,
            events: event_rx,
            art,
            thread: Some(thread),
            offline: false,
            #[cfg(test)]
            network_statuses: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            playlist_item_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            #[cfg(test)]
            playlist_add_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            #[cfg(test)]
            #[cfg(test)]
            #[cfg(test)]
            queue_requests: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            sent_api: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            queued_tracks: std::sync::Mutex::new(Vec::new()),
            #[cfg(all(test, target_os = "linux"))]
            player_loads: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            #[cfg(test)]
            #[cfg(test)]
            #[cfg(test)]
            library_cache_stores: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            home_cache_stores: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Stops service-bound commands from leaving the process; artwork and
    /// shutdown still work. Used by the demo mode and by headless tests.
    #[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn auth_epoch(&self) -> u64 {
        self.auth_epoch.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn send(&self, command: Command) {
        use std::sync::atomic::Ordering;
        #[cfg(all(test, target_os = "linux"))]
        if let Command::Player(PlayerCommand::Load(spec)) = &command {
            self.player_loads.lock().unwrap().push(spec.clone());
        }
        // Invalidate at the UI boundary, not later when the worker happens to
        // drain its queue. This also rejects terminal events already queued.
        let command = match command {
            Command::SignIn { config, .. } => {
                let _ = self.commands.send(Command::UpdateProxy(config));
                Command::AuthStart {
                    attempt: self.auth_epoch.fetch_add(1, Ordering::AcqRel) + 1,
                    restore: false,
                }
            }
            command @ (Command::CancelSignIn | Command::SignOut) => {
                self.auth_epoch.fetch_add(1, Ordering::AcqRel);
                command
            }
            command => command,
        };
        #[cfg(test)]
        if let Command::StoreLibraryCache(cache) = &command {
            self.library_cache_stores
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(cache.clone());
        }
        #[cfg(test)]
        if let Command::StoreHomeCache(cache) = &command {
            self.home_cache_stores
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(cache.clone());
        }
        if self.offline {
            let allowed = match &command {
                Command::Accent { .. } | Command::Shutdown | Command::InspectUpdate => true,
                Command::CheckForUpdates { source, .. }
                | Command::DownloadUpdate { source, .. } => !source.is_github(),
                // In particular, a fixture must never hand off the real test
                // executable to an install helper.
                _ => false,
            };
            if !allowed {
                return;
            }
        }
        let _ = self.commands.send(command);
    }

    /// Construct the native dialog on the UI thread, then await and read it
    /// on the runtime. AppKit requires its window lookup on the main thread.
    pub fn choose_playlist_cover(&self, id: String, request: u64) {
        if self.offline {
            return;
        }
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Choose playlist cover")
            .add_filter("JPEG or PNG image", &["jpg", "jpeg", "png"])
            .pick_file();
        self.send(Command::ChoosePlaylistCover {
            id,
            request,
            selected: Box::pin(selected),
        });
    }

    pub fn api(&self, request: ApiRequest) {
        #[cfg(test)]
        self.sent_api
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::AlbumQueueTracks { .. }
                | ApiRequest::AddToQueue { .. }
                | ApiRequest::AddManyToQueue { .. }
        ) {
            self.queue_requests.lock().unwrap().push(request.clone());
        }
        #[cfg(test)]
        if matches!(
            request,
            ApiRequest::AddToPlaylist { .. }
                | ApiRequest::CheckPlaylistDuplicates { .. }
                | ApiRequest::UpdatePlaylist { .. }
        ) {
            self.playlist_add_requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request.clone());
        }
        #[cfg(test)]
        if let ApiRequest::PlaylistItems {
            id,
            offset,
            generation,
        } = &request
        {
            self.playlist_item_requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((id.clone(), *offset, *generation));
        }
        self.send(Command::Api(request));
    }

    #[cfg(test)]
    pub(crate) fn take_library_cache_stores(
        &self,
    ) -> Vec<crate::account_cache::Cache<Vec<Playlist>>> {
        std::mem::take(
            &mut *self
                .library_cache_stores
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_home_cache_stores(
        &self,
    ) -> Vec<crate::account_cache::Cache<crate::model::HomeSnapshot>> {
        std::mem::take(
            &mut *self
                .home_cache_stores
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    #[cfg(test)]
    pub fn take_playlist_item_requests(&self) -> Vec<(String, u32, u64)> {
        std::mem::take(
            &mut *self
                .playlist_item_requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    #[cfg(test)]
    #[cfg(test)]
    pub fn take_playlist_add_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(
            &mut *self
                .playlist_add_requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    #[cfg(test)]
    pub(crate) fn take_queue_requests(&self) -> Vec<ApiRequest> {
        std::mem::take(&mut *self.queue_requests.lock().unwrap())
    }
    #[cfg(test)]
    pub(crate) fn take_sent_api(&self) -> Vec<ApiRequest> {
        std::mem::take(&mut *self.sent_api.lock().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn take_queued_tracks(&self) -> Vec<String> {
        std::mem::take(&mut *self.queued_tracks.lock().unwrap())
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn take_player_loads(&self) -> Vec<crate::player::LoadSpec> {
        std::mem::take(&mut *self.player_loads.lock().unwrap())
    }

    pub fn player(&self, command: PlayerCommand) {
        #[cfg(test)]
        if let PlayerCommand::AddToQueue(uri) = &command {
            self.queued_tracks.lock().unwrap().push(uri.clone());
        }
        self.send(Command::Player(command));
    }

    /// Whether the system says the internet can be reached. Without it the
    /// daemon waits for it to come back instead of retrying.
    pub fn network_status(&self, reachable: bool) {
        #[cfg(test)]
        self.network_statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(reachable);
        self.send(Command::NetworkStatus(reachable));
    }

    #[cfg(test)]
    pub fn take_network_statuses(&self) -> Vec<bool> {
        std::mem::take(
            &mut *self
                .network_statuses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn art(&self) -> &ArtLoader {
        &self.art
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
