//! Acceptance checks on the real egui output, without a window or GPU.
use super::*;
use crate::api::models::{CursorPage, Image, PlayHistory, SavedTrack};
use std::io::{BufRead, Read, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const CHILD: &str = "MELBA_HEADLESS_TEST_CHILD";
const VIEWPORT: egui::Vec2 = egui::vec2(1000.0, 720.0);

/// Isolate the transport before App::new can start its polling worker. The
/// child has neither a display nor the real account's HOME/XDG directories.
fn isolated() -> bool {
    let thread = std::thread::current();
    run_isolated(
        thread
            .name()
            .expect("the Rust harness names each test thread"),
    )
}

fn run_isolated(test: &str) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(test) {
        return false;
    }
    let root = Scratch(std::env::temp_dir().join(format!(
        "melba-headless-child-{:016x}",
        rand::random::<u64>()
    )));
    for part in ["config", "data", "cache", "state", "run", "tmp"] {
        std::fs::create_dir_all(root.0.join(part)).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    for directory in [&root.0, &root.0.join("run")] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = std::process::Command::new("timeout")
        .args(["30s", "dbus-run-session", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--color", "never"])
        .env(CHILD, test)
        .env("HOME", &root.0)
        .env("XDG_CONFIG_HOME", root.0.join("config"))
        .env("XDG_DATA_HOME", root.0.join("data"))
        .env("XDG_CACHE_HOME", root.0.join("cache"))
        .env("XDG_STATE_HOME", root.0.join("state"))
        .env("XDG_RUNTIME_DIR", root.0.join("run"))
        .env("TMPDIR", root.0.join("tmp"))
        .env(
            "DBUS_SYSTEM_BUS_ADDRESS",
            format!("unix:path={}/no-system-bus", root.0.display()),
        )
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("timeout and dbus-run-session are required on Linux");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
        "isolated child must execute exactly one passing test\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
#[should_panic(expected = "isolated child must execute exactly one passing test")]
fn isolation_rejects_an_empty_child_filter_instead_of_reporting_green() {
    run_isolated("no-such-melba-headless-test");
}

struct TestUi {
    app: App,
    ctx: egui::Context,
    // Drop the app and its file writer before deleting its private directories.
    _scratch: Scratch,
}
impl TestUi {
    fn new() -> Self {
        assert!(
            std::env::var_os(CHILD).is_some(),
            "isolate HOME and the bus before starting the worker"
        );
        let root =
            std::env::temp_dir().join(format!("melba-headless-{:016x}", rand::random::<u64>()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        dirs.ensure().unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        egui_extras::install_image_loaders(&ctx);
        let waker = Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings {
                language: crate::settings::LanguageChoice::Locale(Locale::English),
                proxy_mode: crate::settings::ProxyMode::Off,
                ..Default::default()
            },
            AppOptions {
                media_controls: false,
                restore_sign_in: false,
                tray: false,
            },
        );
        // Only the image loader's loopback HTTP requests may leave this fixture.
        app.backend.set_offline(true);
        app.offline = true;
        app.auth = AuthStatus::Connected {
            username: "fixture".into(),
        };
        app.reveal_theme_changes = false;
        app.attach(&ctx);
        Self {
            app,
            ctx,
            _scratch: Scratch(root),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT)),
                events,
                ..Default::default()
            },
            |ui| self.app.frame_ui(ui),
        )
    }

    fn tree(&mut self, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
        let mut output = self.frame(events);
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .expect("screen-reader tree")
    }
}

impl Drop for TestUi {
    fn drop(&mut self) {
        self.app.backend.shutdown();
    }
}

/// A real HTTP boundary: malformed rendition paths fail rather than yielding
/// an image anyway. All credentials and account data in these tests are fake.
struct ArtworkServer {
    base: String,
    paths: Arc<Mutex<Vec<String>>>,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ArtworkServer {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let paths = Arc::new(Mutex::new(Vec::new()));
        let received = paths.clone();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let (mut socket, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("artwork listener: {e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let complete = {
                    let mut reader = std::io::BufReader::new((&mut socket).take(8192));
                    loop {
                        match reader.read_until(b'\n', &mut request) {
                            Ok(0) | Err(_) => break false, // EOF, limit or timeout.
                            Ok(_) if request.ends_with(b"\r\n\r\n") => break true,
                            Ok(_) => {}
                        }
                    }
                };
                let text = String::from_utf8_lossy(&request);
                let path = if complete {
                    text.split_whitespace().nth(1).unwrap_or("")
                } else {
                    "<incomplete headers>"
                }
                .to_owned();
                received.lock().unwrap().push(path.clone());
                let rendition = path.rsplit('/').next().unwrap_or("");
                let valid = [64, 256, 300, 512, 640]
                    .iter()
                    .any(|n| rendition == format!("{n}x{n}bb.jpg"));
                let color = if !valid {
                    None
                } else if path.starts_with("/avatar/") {
                    Some(AVATAR)
                } else if path.starts_with("/home/") {
                    Some(HOME)
                } else if path.starts_with("/library/") {
                    Some(LIBRARY)
                } else {
                    None
                };
                let (status, bytes) = if let Some(color) = color {
                    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba(color.to_array()));
                    let mut bytes = std::io::Cursor::new(Vec::new());
                    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
                    ("200 OK", bytes.into_inner())
                } else {
                    ("400 Bad Request", Vec::new())
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = socket
                    .write_all(header.as_bytes())
                    .and_then(|()| socket.write_all(&bytes));
            }
        });
        Self {
            base,
            paths,
            done,
            thread: Some(thread),
        }
    }
}
impl Drop for ArtworkServer {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

const AVATAR: Color32 = Color32::from_rgb(17, 93, 201);
const HOME: Color32 = Color32::from_rgb(201, 73, 17);
const LIBRARY: Color32 = Color32::from_rgb(39, 171, 83);

fn song(base: &str, id: &str, kind: &str, title: &str) -> Track {
    crate::mk::song(&serde_json::json!({
        "id": id, "type": kind,
        "attributes": {
            "name": title, "artistName": "Fixture artist", "durationInMillis": 180000,
            "artwork": {"url": format!("{base}/{{w}}x{{h}}{{c}}.{{f}}")}
        }
    }))
}

#[test]
fn api_artwork_reaches_visible_avatar_home_and_library_meshes_without_a_window() {
    if isolated() {
        return;
    }
    let server = ArtworkServer::new();
    let mut ui = TestUi::new();
    ui.app.handle_api(ApiResponse::Me(Ok(User {
        id: "fixture".into(),
        display_name: Some("Fixture user".into()),
        images: vec![Image {
            url: crate::mk::artwork_url(
                &format!("{}/avatar/{{w}}x{{h}}{{c}}.{{f}}", server.base),
                512,
                512,
            ),
            width: Some(512),
            height: Some(512),
        }],
        ..Default::default()
    })));
    ui.app.home.made_for_you = Loadable::Loaded(Vec::new());
    ui.app.home.top_artists = Loadable::Loaded(Vec::new());
    ui.app.home.top_tracks = Loadable::Loaded(Vec::new());
    ui.app.home.recommendations = Loadable::Loaded(Vec::new());
    let home = song(
        &format!("{}/home", server.base),
        "home",
        "songs",
        "Home fixture",
    );
    assert!(home.album.is_none());
    ui.app.handle_api(ApiResponse::RecentlyPlayed {
        who: RecentsFor::Home,
        generation: ui.app.home.generation,
        limit: 20,
        result: Ok(CursorPage {
            items: vec![PlayHistory {
                track: home,
                ..Default::default()
            }],
            ..Default::default()
        }),
    });
    let library = song(
        &format!("{}/library", server.base),
        "library",
        "library-songs",
        "Library fixture",
    );
    ui.app.handle_api(ApiResponse::SavedTracks {
        offset: 0,
        generation: ui.app.liked_songs.generation,
        account_id: Some("fixture".into()),
        result: Ok(crate::api::models::Page {
            items: vec![SavedTrack {
                track: library,
                ..Default::default()
            }],
            total: 1,
            ..Default::default()
        }),
    });

    let mut textures = HashMap::<egui::TextureId, HashSet<Color32>>::new();
    for (page, expected) in [(Page::Home, HOME), (Page::LikedSongs, LIBRARY)] {
        ui.app.open(page.clone());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut output = ui.frame(Vec::new());
            for (id, deltas) in &output.textures_delta.set {
                for delta in deltas {
                    // Image textures are full uploads; partial font-atlas updates
                    // are not evidence of any of the fixture's cover images.
                    if delta.pos.is_none() {
                        let egui::ImageData::Color(image) = &delta.image;
                        let colors = [AVATAR, HOME, LIBRARY]
                            .into_iter()
                            .filter(|color| image.pixels.contains(color))
                            .collect();
                        textures.insert(*id, colors);
                    }
                }
            }
            // We inspected the uploads above; there is deliberately no GPU
            // renderer here to consume the deltas.
            output.textures_delta.clear();
            let mut visible = HashSet::new();
            for primitive in ui.ctx.tessellate(output.shapes, output.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    let bounds = mesh
                        .vertices
                        .iter()
                        .fold(egui::Rect::NOTHING, |mut r, vertex| {
                            r.extend_with(vertex.pos);
                            r
                        });
                    if bounds.intersects(primitive.clip_rect)
                        && mesh.vertices.iter().any(|vertex| vertex.color.a() != 0)
                        && let Some(colors) = textures.get(&mesh.texture_id)
                    {
                        visible.extend(colors.iter().copied());
                    }
                }
            }
            if visible.contains(&AVATAR) && visible.contains(&expected) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{page:?}: expected avatar and artwork pixels in visible meshes, got {visible:?}; HTTP paths: {:?}",
                server.paths.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let paths = server.paths.lock().unwrap();
    for part in ["/avatar/", "/home/", "/library/"] {
        assert!(
            paths
                .iter()
                .any(|p| p.starts_with(part) && p.ends_with("bb.jpg")),
            "missing rendered request for {part}: {paths:?}"
        );
    }
}

#[test]
fn accessible_second_album_row_changes_the_player_and_preserves_the_load_offset() {
    if isolated() {
        return;
    }
    select_second_album_row(false);
}

#[test]
fn double_clicking_second_album_row_changes_the_player_and_preserves_the_load_offset() {
    if isolated() {
        return;
    }
    select_second_album_row(true);
}

fn select_second_album_row(pointer: bool) {
    let server = ArtworkServer::new();
    let mut ui = TestUi::new();
    ui.app.handle_api(ApiResponse::Me(Ok(User {
        id: "fixture".into(),
        ..Default::default()
    })));
    let tracks: Vec<_> = [
        ("first", "First fixture"),
        ("second", "Second fixture"),
        ("third", "Third fixture"),
    ]
    .into_iter()
    .map(|(id, title)| song(&format!("{}/library", server.base), id, "songs", title))
    .collect();
    ui.app.local_ready = true;
    ui.app.local = LocalState {
        connected: true,
        playback: Playback::Playing,
        track: Some(crate::player::LocalTrack {
            uri: tracks[0].uri.clone(),
            title: tracks[0].name.clone(),
            artists: tracks[0].artists.clone(),
            duration_ms: 180000,
            ..Default::default()
        }),
        ..Default::default()
    };
    ui.app.open(Page::Album("fixture-album".into()));
    ui.app.handle_api(ApiResponse::Album {
        id: "fixture-album".into(),
        result: Ok(Album {
            id: "fixture-album".into(),
            uri: "melba:album:fixture-album".into(),
            name: "Fixture album".into(),
            total_tracks: Some(3),
            tracks: Some(crate::api::models::Page {
                items: tracks.clone(),
                total: 3,
                ..Default::default()
            }),
            ..Default::default()
        }),
    });
    ui.tree(Vec::new());
    let tree = ui.tree(Vec::new());
    let (second, row) = tree
        .nodes
        .iter()
        .find(|(_, node)| {
            node.role() == egui::accesskit::Role::Button
                && node.label() == Some("Play Second fixture, Fixture artist")
        })
        .map(|(id, node)| (*id, node.bounds().unwrap()))
        .expect("the second song must expose an actionable accessible row");
    assert_eq!(ui.app.now_playing().unwrap().uri, tracks[0].uri);
    assert!(ui.app.backend.take_player_loads().is_empty());
    if pointer {
        let pos = egui::pos2(
            ((row.x0 + row.x1) / 2.0) as f32,
            ((row.y0 + row.y1) / 2.0) as f32,
        );
        // Raw events go through egui's actual hit testing, not a direct
        // invocation of the row's action handler or the desktop pointer.
        for _ in 0..2 {
            ui.tree(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    } else {
        ui.tree(vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: second,
                action: egui::accesskit::Action::Click,
                data: None,
            },
        )]);
    }
    let mut output = ui.frame(Vec::new());
    output.textures_delta.clear();
    assert_eq!(
        ui.app.now_playing().unwrap().uri,
        tracks[1].uri,
        "the old playing song must not win over the click"
    );
    let loads = ui.app.backend.take_player_loads();
    assert_eq!(
        loads.len(),
        1,
        "one row activation must request exactly one load"
    );
    assert_eq!(
        loads[0].context_uri.as_deref(),
        Some("spotify:album:fixture-album")
    );
    assert_eq!(loads[0].offset_uri.as_deref(), Some("spotify:track:second"));
    assert!(
        loads[0].uris.is_empty(),
        "context resolution must not be bypassed by an unrelated explicit list"
    );
    assert_eq!(loads[0].offset_index, None);
    assert!(loads[0].play);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| playing_title(&s.shape, s.clip_rect)),
        "the player bar must draw the selected song, not merely keep it in the album table"
    );
}

fn playing_title(shape: &egui::epaint::Shape, clip: egui::Rect) -> bool {
    match shape {
        egui::epaint::Shape::Text(text) => {
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT);
            let bounds = text.visual_bounding_rect();
            text.galley.job.text == "Second fixture"
                && bounds.min.y > viewport.max.y - 100.0
                && bounds.is_positive()
                && clip.intersect(viewport).contains_rect(bounds)
                && text.opacity_factor > 0.0
                && text.galley.rows.iter().any(|placed| {
                    let visuals = &placed.row.visuals;
                    visuals.mesh.vertices[visuals.glyph_vertex_range.clone()]
                        .iter()
                        .any(|vertex| {
                            let color = text.override_text_color.unwrap_or_else(|| {
                                if vertex.color == Color32::PLACEHOLDER {
                                    text.fallback_color
                                } else {
                                    vertex.color
                                }
                            });
                            color.gamma_multiply(text.opacity_factor.min(1.0)).a() != 0
                        })
                })
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|shape| playing_title(shape, clip)),
        _ => false,
    }
}

#[test]
fn player_title_probe_rejects_offscreen_clipped_and_transparent_text() {
    if isolated() {
        return;
    }
    let ctx = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT);
    let mut text = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        },
        |ui| {
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    "Second fixture".into(),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                )
            });
            text = Some(egui::epaint::TextShape::new(
                egui::pos2(300.0, 650.0),
                galley,
                Color32::WHITE,
            ));
        },
    );
    output.textures_delta.clear();
    let mut text = text.unwrap();
    let visible = |text: &egui::epaint::TextShape, clip| {
        playing_title(&egui::epaint::Shape::Text(text.clone()), clip)
    };
    assert!(visible(&text, viewport));
    text.pos.y = 740.0;
    assert!(!visible(&text, viewport), "offscreen title must not count");
    text.pos.y = 650.0;
    let above_player = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1000.0, 600.0));
    assert!(
        !visible(&text, above_player),
        "clipped title must not count"
    );
    text.opacity_factor = 0.0;
    assert!(
        !visible(&text, viewport),
        "transparent title must not count"
    );
    text.opacity_factor = 1.0;
    text.override_text_color = Some(Color32::TRANSPARENT);
    assert!(
        !visible(&text, viewport),
        "transparent glyphs must not count"
    );
}

/// The daemon can only name a queue row by the URI it will open. The panel
/// must show the song this session already loaded for that URI.
#[test]
fn queue_rows_show_the_loaded_song_instead_of_the_daemon_uri() {
    if isolated() {
        return;
    }
    let mut ui = TestUi::new();
    ui.app.handle_api(ApiResponse::Me(Ok(User {
        id: "fixture".into(),
        ..Default::default()
    })));
    ui.app.local_ready = true;
    ui.app.local = LocalState {
        connected: true,
        playback: Playback::Paused,
        ..Default::default()
    };
    let id = "1486853005";
    let known = song(
        "https://fixture.invalid/queue",
        id,
        "songs",
        "Queue fixture",
    );
    ui.app.track_cache.insert(id.to_owned(), known);
    ui.app.queue = Loadable::Loaded(Queue {
        currently_playing: None,
        queue: vec![PlayableItem::Track(crate::api::models::Track {
            uri: format!("stream:{id}"),
            name: format!("stream:{id}"),
            ..Default::default()
        })],
    });
    ui.app.queue_tab = QueueTab::Queue;
    ui.app.show_queue_panel = true;
    let tree = ui.tree(Vec::new());
    let labels: Vec<String> = tree
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label == "Play Queue fixture, Fixture artist"),
        "queue row must show the loaded song: {labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|label| label.contains("stream:1486853005")),
        "a playable URI is not a song name: {labels:?}"
    );
}
