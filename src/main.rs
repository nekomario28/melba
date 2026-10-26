//! melbad — native lightweight music client for Linux.
//! Slice 01 surface: `melbad play <file> [--verify] [--device auto|hw:X,Y]`
//! and `melbad probe`.

use melbad::audio::{self, open_auto, open_exact, Granted, OpenError, SourceFormat};
use melbad::{auth, library, musickit, player, stream};

const BOOKMARKLET: &str = "javascript:(()=>{try{var t=MusicKit.getInstance().musicUserToken;if(!t)throw 0;prompt('Copy this token for `melbad auth set`:',t)}catch(e){alert('Sign in to Apple Music first, then click this again.')}})()";
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "melbad",
    version,
    about = "native lightweight music client (bit-perfect core)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List PCM playback devices and their negotiated capability bounds.
    Probe {
        /// Also show slots that failed to open (phantom subdevices etc).
        #[arg(long)]
        all: bool,
        /// Machine-readable array of DeviceCap.
        #[arg(long)]
        json: bool,
    },
    /// Import a playlist/dir into a sone.collection/v1 JSON document.
    Import {
        /// .xspf/.m3u/iTunes .xml file, or a directory to scan.
        path: String,
        /// Validate output against this schema file.
        #[arg(long)]
        schema: Option<String>,
    },
    /// Scan a directory of audio files into a sone.collection/v1 library.
    Scan {
        dir: String,
        #[arg(long)]
        schema: Option<String>,
    },
    /// MPRIS daemon: serve org.mpris.MediaPlayer2.melba over a queue.
    Serve {
        files: Vec<String>,
        /// hw:X,Y, auto, or default. Omitted: audio_device from config, else auto.
        #[arg(long)]
        device: Option<String>,
        /// Refuse the OS mixer (`default`); auto only opens an exact hw device.
        #[arg(long)]
        require_bitperfect: bool,
        /// Prepare a restored queue without starting playback (desktop startup).
        #[arg(long)]
        paused: bool,
    },
    /// Send a transport verb to the daemon:
    /// play/pause/playpause/stop/next/previous/status
    /// seek <+/-sec|ms> · setpos <sec> · loop <none|playlist|track> · shuffle <on|off> · open <file://uri|path|stream:<song-id>> · enqueue <uri...> · load <index> <uri...>
    /// volume <0..100> · device <hw:X,Y|auto> · reorder|move <from> <to> · goto <index> · clear · quit
    Ctl { verb: Vec<String> },
    /// MusicKit catalog metadata (public developer token, no login).
    Mk {
        #[command(subcommand)]
        op: MkOp,
    },
    /// Apple account: sign in, inspect or remove the saved MusicKit user token.
    Auth {
        #[command(subcommand)]
        op: AuthOp,
    },
    /// Stream the catalog preview (non-DRM 30s AAC) for a song id.
    Preview { id: String },
    /// Download catalog preview + artwork + manifest to the cache dir.
    Dl { id: String },
    /// Full player state as one JSON object (agent-friendly).
    Now,
    /// Capability manifest as JSON — what this binary can do.
    About,
    /// Diagnose the subscription-playback chain: token, developer token,
    /// Widevine CDM, ALAC wrapper, audio output.
    Doctor,
    /// Managed ALAC decrypt service: setup (fetch + extract), start,
    /// stop, status. Lives under ~/.local/share/melba/wrapper/.
    Wrapper {
        #[command(subcommand)]
        op: WrapOp,
    },
    /// Local configuration (audio output). No credential values.
    Config {
        #[command(subcommand)]
        op: ConfigOp,
    },
    /// First-run audio setup: choose an output and optionally check the wrapper.
    Setup {
        /// Accept the recommended device without prompting.
        #[arg(long)]
        yes: bool,
    },
    /// Play a file; with --verify, emit the format receipt JSON.
    Play {
        file: String,
        /// hw:X,Y, auto, or default. Omitted: audio_device from config, else auto.
        #[arg(long)]
        device: Option<String>,
        /// Refuse `default` (the OS mixer); no silent shared fallback.
        #[arg(long)]
        require_bitperfect: bool,
        /// Emit evidence/slice01-format.json-style receipt to stdout.
        #[arg(long)]
        verify: bool,
        /// Stop after N seconds of playback (verify helper).
        #[arg(long)]
        max_seconds: Option<u64>,
    },
}

#[derive(Subcommand)]
enum ConfigOp {
    /// Get or set audio_device (auto | default | hw:X,Y).
    AudioDevice {
        /// Omit to print the current value.
        value: Option<String>,
    },
}

#[derive(Serialize)]
struct FormatReceipt {
    source: SourceFormat,
    granted: Option<Granted>,
    verdict: String,
    frames_written: u64,
    elapsed_ms: u128,
    peak_rss_kb: u64,
    notes: Vec<String>,
}

fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn resolve_device(device: Option<String>) -> String {
    device
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .or_else(melbad::config::audio_device)
        .unwrap_or_else(|| "auto".into())
}

fn refuse_shared(device: &str, require_bitperfect: bool) -> bool {
    if require_bitperfect && device == "default" {
        eprintln!("--require-bitperfect refuses `default` (the OS mixer)");
        return false;
    }
    true
}

fn run_play(file: &str, device: &str, verify: bool, max_seconds: Option<u64>) -> i32 {
    let started = Instant::now();
    // one open+probe feeds format detection AND the decode loop
    let pf = match audio::open_source(file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("source probe failed: {e}");
            return 3;
        }
    };
    let src = pf.src.clone();
    eprintln!(
        "source: {}bit {}Hz x{}ch ({})",
        src.bits, src.rate, src.channels, src.codec
    );

    let (pcm, granted) = match if device == "auto" {
        open_auto(&src)
    } else {
        open_exact(device, &src)
    } {
        Ok(ok) => ok,
        Err(OpenError::Busy(m)) => {
            eprintln!("device busy: {m}");
            return 4;
        }
        Err(OpenError::Rejected { device: d, why }) => {
            eprintln!("device rejected: {d}: {why}");
            if verify {
                let r = FormatReceipt {
                    source: src.clone(),
                    granted: None,
                    verdict: "CLAMPED".into(),
                    frames_written: 0,
                    elapsed_ms: started.elapsed().as_millis(),
                    peak_rss_kb: peak_rss_kb(),
                    notes: vec![format!("{d}: {why}")],
                };
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            }
            return 5;
        }
        Err(OpenError::NoDevice { tried }) => {
            eprintln!("no device can carry the source format exactly:");
            for t in &tried {
                eprintln!("  {t}");
            }
            // Honest negative: nothing could play it at source format.
            if verify {
                let r = FormatReceipt {
                    source: src.clone(),
                    granted: None,
                    verdict: "CLAMPED".into(),
                    frames_written: 0,
                    elapsed_ms: started.elapsed().as_millis(),
                    peak_rss_kb: peak_rss_kb(),
                    notes: tried.clone(),
                };
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            }
            return 6;
        }
    };
    eprintln!(
        "granted: {} {} {}bit(msbits) x{}ch",
        granted.device, granted.rate, granted.msbits, granted.channels
    );

    let v = audio::verdict_str(&src, &granted);
    if v != "BITPERFECT" {
        eprintln!("verdict: {v} — refusing to play as bit-perfect");
    }

    let t0 = Instant::now();
    let frames = if v == "BITPERFECT" {
        let cap = max_seconds.map(|s| s.saturating_mul(src.rate as u64));
        match audio::play_into(&pcm, &granted, pf, cap) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("playback: {e}");
                0
            }
        }
    } else {
        0
    };
    drop(pcm);

    if verify {
        let mut notes = Vec::new();
        if let Some(s) = max_seconds {
            notes.push(format!("max_seconds={s} enforced"));
        }
        notes.push(
            if v == "BITPERFECT" {
                "exclusive hw: open — no graph resampler can sit between us and the device"
            } else {
                "output does not establish an exclusive bit-perfect path; playback refused"
            }
            .into(),
        );
        let r = FormatReceipt {
            source: src.clone(),
            granted: Some(granted.clone()),
            verdict: v.to_string(),
            frames_written: frames,
            elapsed_ms: t0.elapsed().as_millis(),
            peak_rss_kb: peak_rss_kb(),
            notes,
        };
        let json = serde_json::to_string_pretty(&r).unwrap();
        println!("{json}");
        // receipt persistence is opt-in — writing into whatever cwd the
        // caller happened to be in was an invisible side effect
        if let Some(dir) = std::env::var_os("MELBA_EVIDENCE_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(Path::new(&dir).join("slice01-format.json"), &json);
        }
    }
    if v == "BITPERFECT" {
        0
    } else {
        7
    }
}

#[derive(Subcommand)]
enum WrapOp {
    /// Download the wrapper release and extract Apple Music libs from
    /// an APK (auto-fetch unless --apk points at a local file).
    Setup { apk: Option<String> },
    /// One-time Apple ID login so the device/account DB is provisioned;
    /// afterwards the service runs without credentials. The password is
    /// read from the terminal with echo off, from stdin when it is not a
    /// terminal, or from MELBA_WRAPPER_PASSWORD. It is not an argument.
    Login {
        user: String,
        /// Kept so a leftover password argument is rejected without clap
        /// echoing the value in its usage error.
        #[arg(hide = true, allow_hyphen_values = true)]
        pass: Option<String>,
    },
    /// Start the managed service in the foreground parent (kept by the
    /// daemon during playback; mainly for debugging).
    Start,
    /// Stop the managed service.
    Stop,
    /// Install dir, binary, and key-port reachability.
    Status,
}

#[derive(Subcommand)]
enum MkOp {
    /// Print the Apple developer token fetched from music.apple.com.
    Token,
    /// Search the catalog (types: songs,albums,artists,playlists).
    Search {
        term: String,
        #[arg(long, default_value = "songs,albums")]
        types: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Emit as sone.collection/v1 with preview:<id> playUris (serve-ready).
        #[arg(long)]
        collection: bool,
    },
    /// Emit an album's or playlist's tracks as a serve-ready collection.
    Collection {
        /// albums | playlists
        kind: String,
        id: String,
    },
    /// Album detail.
    Album { id: String },
    /// Artist detail: header + full albums + top songs.
    Artist { id: String },
    /// Song detail incl. audioVariants (lossless / hi-res-lossless flags).
    Song { id: String },
    /// Enrich a sone.collection JSON doc on stdin/file with apple.* props.
    Enrich {
        /// Collection JSON file (or "-" for stdin).
        file: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// User library / history / recommendations (needs `melbad auth open`).
    Me {
        /// library-albums|library-songs|library-playlists|recent-tracks|
        /// recent-albums|recommendations|heavy-rotation
        what: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Add a catalog item to the user's library: mk add songs|albums <id>
    Add { rtype: String, id: String },
    /// Lyrics for a song (needs `melbad auth open` — catalog token lacks the permission).
    Lyrics { id: String },
}

#[derive(Subcommand)]
enum AuthOp {
    /// Private GUI protocol on stdin/stdout. Uses native wrapper prompts when installed.
    Gui {
        #[arg(long)]
        restore: bool,
    },
    /// Store the user's music-user-token (see `melbad auth`).
    Set { token: String },
    /// Show whether a token is stored and its exp if decodable.
    Status,
    /// Remove the stored token and any cached wrapper binding.
    Clear,
    /// Print how to obtain the token.
    How,
    /// Sign in inside a Melba window (music.apple.com); the session
    /// cookie is captured and stored automatically once Apple issues it.
    /// --browser instead opens your normal browser and watches its
    /// localStorage for the token.
    Open {
        #[arg(long)]
        browser: bool,
    },
}

fn send_mpris_verb(argv: &[String]) -> Result<(), String> {
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;
    use std::time::Duration;
    let verb = argv.first().map(|s| s.as_str()).unwrap_or("status");
    let arg = argv.get(1).map(|s| s.as_str());
    let conn = Connection::new_session().map_err(|e| format!("dbus: {e}"))?;
    let proxy = conn.with_proxy(
        "org.mpris.MediaPlayer2.melba",
        "/org/mpris/MediaPlayer2",
        Duration::from_secs(3),
    );
    let iface = "org.mpris.MediaPlayer2.Player";
    match verb {
        "status" => {
            let st: String = proxy
                .get(iface, "PlaybackStatus")
                .map_err(|e| e.to_string())?;
            println!("{st}");
            return Ok(());
        }
        "seek" => {
            // +10 / -5 seconds, or raw µs if suffix "us"
            let a = arg.ok_or("seek needs an arg")?;
            let us: i64 = if let Some(u) = a.strip_suffix("us") {
                u.parse().map_err(|_| "seek arg")?
            } else {
                (a.parse::<f64>().map_err(|_| "seek arg")? * 1e6) as i64
            };
            proxy
                .method_call::<(), _, _, _>(iface, "Seek", (us,))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "setpos" => {
            let s: f64 = arg
                .ok_or("setpos needs seconds")?
                .parse()
                .map_err(|_| "setpos arg")?;
            // SetPosition needs the REAL current trackid — the daemon
            // ignores a stale object path per spec, so a fabricated one
            // ("/org/mpris/.../Track/current") silently no-ops.
            let meta: dbus::arg::PropMap =
                proxy.get(iface, "Metadata").map_err(|e| e.to_string())?;
            let tid = meta
                .get("mpris:trackid")
                .and_then(|v| dbus::arg::cast::<dbus::Path>(&*v.0))
                .cloned()
                .ok_or("no mpris:trackid in Metadata")?;
            proxy
                .method_call::<(), _, _, _>(iface, "SetPosition", (tid, (s * 1e6) as i64))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "loop" => {
            let m = match arg.unwrap_or("none") {
                "playlist" => "Playlist",
                "track" => "Track",
                _ => "None",
            };
            proxy
                .set(iface, "LoopStatus", m)
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "shuffle" => {
            let on = matches!(arg.unwrap_or("on"), "on" | "true" | "1");
            proxy.set(iface, "Shuffle", on).map_err(|e| e.to_string())?;
            return Ok(());
        }
        "volume" | "vol" => {
            // "60" = 60%, "0.6" = 60% — bare 0..=1 also accepted
            let a = arg.ok_or("volume needs 0..100")?;
            let f: f64 = a.parse().map_err(|_| "volume arg")?;
            let v = if f > 1.0 { f / 100.0 } else { f }.clamp(0.0, 1.0);
            proxy.set(iface, "Volume", v).map_err(|e| e.to_string())?;
            return Ok(());
        }
        "open" => {
            let p = arg.ok_or("open needs a path")?;
            let uri = if p.starts_with("file://")
                || p.starts_with("preview:")
                || p.starts_with("stream:")
            {
                p.to_string()
            } else {
                format!(
                    "file://{}",
                    std::fs::canonicalize(p)
                        .map_err(|e| e.to_string())?
                        .display()
                )
            };
            proxy
                .method_call::<(), _, _, _>(iface, "OpenUri", (uri,))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "enqueue" | "add" => {
            // append without disturbing the current track
            let mut uris = Vec::new();
            for p in argv.iter().skip(1) {
                uris.push(
                    if p.starts_with("file://")
                        || p.starts_with("preview:")
                        || p.starts_with("stream:")
                    {
                        p.to_string()
                    } else {
                        format!(
                            "file://{}",
                            std::fs::canonicalize(p)
                                .map_err(|e| e.to_string())?
                                .display()
                        )
                    },
                );
            }
            if uris.is_empty() {
                return Err("enqueue needs a path, preview:<id> or stream:<id>".into());
            }
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaEnqueue", (uris,))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "load" => {
            let index: u32 = arg
                .ok_or("load needs <index> <uri...>")?
                .parse()
                .map_err(|_| "load index must be a non-negative integer")?;
            let uris: Vec<String> = argv
                .iter()
                .skip(2)
                .map(|p| {
                    if p.starts_with("file://")
                        || p.starts_with("preview:")
                        || p.starts_with("stream:")
                    {
                        Ok(p.clone())
                    } else {
                        std::fs::canonicalize(p)
                            .map(|p| format!("file://{}", p.display()))
                            .map_err(|e| e.to_string())
                    }
                })
                .collect::<Result<_, String>>()?;
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaLoadQueue", (uris, index, 0u32, true))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "device" => {
            // runtime output switch: hw:X,Y or auto; restarts the current
            // track on the newly granted sink
            let d = arg.ok_or("device needs hw:X,Y or auto")?;
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaSetDevice", (d,))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "reorder" | "move" => {
            // `melbad ctl reorder <from> <to>` — same contract as the D-Bus
            // method: `to` is the index AFTER removal, so moving 0 to the
            // slot currently at 6 is `reorder 0 6`.
            let from: u32 = argv
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or("reorder needs <from> <to> (queue indexes)")?;
            let to: u32 = argv
                .get(2)
                .and_then(|s| s.parse().ok())
                .ok_or("reorder needs <from> <to> (queue indexes)")?;
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaQueueReorder", (from, to))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "goto" => {
            let i: u32 = argv
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or("goto needs a queue index")?;
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaGoTo", (i,))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        "clear" | "clear-queue" => {
            proxy
                .method_call::<(), _, _, _>(iface, "MelbaQueueClear", ())
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        _ => {}
    }
    let method = match verb {
        "play" => "Play",
        "pause" => "Pause",
        "playpause" | "toggle" => "PlayPause",
        "stop" => "Stop",
        "next" => "Next",
        "prev" | "previous" => "Previous",
        "quit" => {
            return proxy
                .method_call::<(), _, _, _>("org.mpris.MediaPlayer2", "Quit", ())
                .map_err(|e| e.to_string());
        }
        _ => return Err(format!("unknown verb {verb}")),
    };
    proxy
        .method_call::<(), _, _, _>(iface, method, ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Probe { all, json } => {
            let devices: Vec<_> = audio::list_devices()
                .into_iter()
                .filter(|d| all || d.usable())
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&devices).unwrap());
                return;
            }
            println!("{:12} {:28} pick the best sink automatically", "auto", "");
            println!(
                "{:12} {:28} OS mixer (PipeWire/Pulse) — follows system output, not bit-perfect",
                "default", ""
            );
            for d in devices.iter() {
                println!(
                    "{:12} {:28} rates {}..{}Hz  ch {}..{}  {}",
                    d.device,
                    d.card_name,
                    d.rate_min,
                    d.rate_max,
                    d.ch_min,
                    d.ch_max,
                    if d.openable { "open" } else { &d.note }
                );
            }
        }
        Cmd::Serve {
            files,
            device,
            require_bitperfect,
            paused,
        } => {
            let device = resolve_device(device);
            if !refuse_shared(&device, require_bitperfect) {
                std::process::exit(2);
            }
            let shared = player::Shared::new();
            let (tx, rx) = channel::<player::Cmd>();
            // a .json arg holding sone.collection/v1 expands to its playUri entries
            let mut paths: Vec<std::path::PathBuf> = Vec::new();
            for f in &files {
                {
                    if let Ok(txt) = std::fs::read_to_string(f) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                            if v["schema"].as_str() == Some("sone.collection/v1") {
                                if let Some(ts) = v["tracks"].as_array() {
                                    let mut tm =
                                        shared.titles.lock().unwrap_or_else(|e| e.into_inner());
                                    for t in ts {
                                        if let Some(u) = t["playUri"].as_str() {
                                            let raw = u.strip_prefix("file://").unwrap_or(u);
                                            // absolute path so xesam:url / art
                                            // cache keys are stable for any cwd
                                            let pc = std::fs::canonicalize(raw)
                                                .map(|x| x.to_string_lossy().to_string())
                                                .unwrap_or_else(|_| raw.to_string());
                                            let p: &str = &pc;
                                            let label =
                                                match (t["artist"].as_str(), t["title"].as_str()) {
                                                    (Some(a), Some(ti)) => format!("{a} — {ti}"),
                                                    (None, Some(ti)) => ti.to_string(),
                                                    _ => String::new(),
                                                };
                                            if !label.is_empty() {
                                                tm.insert(p.to_string(), label);
                                            }
                                            if let Some(au) = t["props"]
                                                .get("apple.artwork")
                                                .and_then(|x| x.as_str())
                                            {
                                                shared
                                                    .art_urls
                                                    .lock()
                                                    .unwrap_or_else(|e| e.into_inner())
                                                    .insert(p.to_string(), au.to_string());
                                            }
                                            shared
                                                .meta
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .insert(
                                                    p.to_string(),
                                                    player::TrackMeta {
                                                        artist: t["artist"]
                                                            .as_str()
                                                            .map(|s| s.to_string()),
                                                        album: t["album"]
                                                            .as_str()
                                                            .map(|s| s.to_string()),
                                                        art_url: t["props"]
                                                            .get("apple.artwork")
                                                            .and_then(|x| x.as_str())
                                                            .map(|s| s.to_string()),
                                                    },
                                                );
                                            paths.push(p.into());
                                        }
                                    }
                                }
                                continue;
                            }
                        }
                    }
                }
                paths.push(std::fs::canonicalize(f).unwrap_or_else(|_| f.into()));
            }
            // no args: resume the queue the daemon last persisted
            // (~/.local/state/melba/queue.json); explicit files always win.
            if paths.is_empty() && player::restore_queue(&shared) {
                eprintln!("restored queue from XDG_STATE_HOME/melba/queue.json");
            }
            shared
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(paths);
            if let Err(e) =
                player::serve(shared, tx, rx, device.clone(), paused, require_bitperfect)
            {
                eprintln!("serve: {e}");
                std::process::exit(9);
            }
        }
        Cmd::Ctl { verb } => {
            if let Err(e) = send_mpris_verb(&verb) {
                eprintln!("ctl {}: {e}", verb.join(" "));
                std::process::exit(10);
            }
        }
        Cmd::Auth { op } => {
            match op {
                AuthOp::Gui { restore } => {
                    if melbad::auth_gui::run(restore).is_err() {
                        std::process::exit(1);
                    }
                }
                AuthOp::Set { token } => match auth::set(&token) {
                    Ok(()) => println!(
                        "token stored ({})",
                        auth::token_path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                    Err(e) => {
                        eprintln!("auth: {e}");
                        std::process::exit(1);
                    }
                },
                AuthOp::Status => match auth::get() {
                    Some(t) => {
                        let exp = auth::exp_of(&t)
                            .map(|e| {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                let left = e as i64 - now as i64;
                                format!("exp in {}s", left)
                            })
                            .unwrap_or_else(|| "exp unknown".into());
                        // local exp lies when Apple killed the session
                        // server-side — probe the catalog to be sure
                        match musickit::validate_token(&t) {
                            Ok(()) => println!("music-user-token: stored, {exp}, valid"),
                            Err(e) => {
                                println!("music-user-token: stored, {exp}, INVALID ({e})");
                                println!("run `melbad auth open` to sign in again");
                            }
                        }
                    }
                    None => println!("no token stored — see `melbad auth how`"),
                },
                AuthOp::Clear => {
                    match auth::clear().and_then(|()| melbad::auth_gui::forget_binding()) {
                        Ok(()) => println!("cleared"),
                        Err(e) => {
                            eprintln!("auth clear: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                AuthOp::Open { browser } => {
                    if browser {
                        println!(
                            "Opened music.apple.com — sign in there; melba picks up the session automatically (10 min)."
                        );
                        let _ = std::process::Command::new("xdg-open")
                            .arg("https://music.apple.com")
                            .spawn();
                        match auth::watch_browser(
                            std::time::Duration::from_secs(600),
                            musickit::validate_token,
                        ) {
                            Ok(m) => println!("{m}"),
                            Err(e) => {
                                eprintln!("melbad: {e}");
                                std::process::exit(1);
                            }
                        }
                    } else {
                        match melbad::auth_webview::run() {
                            Ok(()) => {}
                            Err(e) => {
                                eprintln!(
                                    "melbad: in-app sign-in unavailable ({e}) — trying the browser watch"
                                );
                                let _ = std::process::Command::new("xdg-open")
                                    .arg("https://music.apple.com")
                                    .spawn();
                                match auth::watch_browser(
                                    std::time::Duration::from_secs(600),
                                    musickit::validate_token,
                                ) {
                                    Ok(m) => println!("{m}"),
                                    Err(e2) => {
                                        eprintln!("melbad: {e2}");
                                        std::process::exit(1);
                                    }
                                }
                            }
                        }
                    }
                }
                AuthOp::How => println!(
                    "`melbad auth open` opens a music.apple.com sign-in window inside\n\
                     melbad — log in once, the session cookie is captured and stored\n\
                     automatically. `melbad auth open --browser` uses your normal\n\
                     browser instead and watches for the token.\n\
                     Manual: `melbad auth set <token>` (devtools:\n\
                     MusicKit.getInstance().musicUserToken), or this bookmarklet\n\
                     in your bookmarks bar — after sign-in one click shows the\n\
                     token to paste into `melbad auth set`:\n\n{}\n\n\
                     Token storage: ~/.config/melba/music-user-token (0600).",
                    BOOKMARKLET
                ),
            }
        }
        Cmd::Preview { id } => match musickit::download_preview(&id) {
            Ok(p) => {
                let code = run_play(&p.to_string_lossy(), "auto", true, None);
                if code != 0 {
                    std::process::exit(code);
                }
            }
            Err(e) => {
                eprintln!("preview: {e}");
                std::process::exit(12);
            }
        },
        Cmd::Dl { id } => match musickit::download(&id) {
            Ok(m) => println!("{}", serde_json::to_string_pretty(&m).unwrap()),
            Err(e) => {
                eprintln!("dl: {e}");
                std::process::exit(12);
            }
        },
        Cmd::Now => {
            if let Err(e) = now_json() {
                eprintln!("now: {e}");
                std::process::exit(10);
            }
        }
        Cmd::About => {
            println!(
                "{}",
                serde_json::to_string_pretty(&about_manifest()).unwrap()
            );
        }
        Cmd::Doctor => {
            let mut bad = 0u32;
            let mut rep = |name: &str, line: Result<String, String>| match line {
                Ok(s) => println!("  {name}: {s}"),
                Err(e) => {
                    bad += 1;
                    println!("  {name}: FAIL {e}");
                }
            };
            println!("subscription playback chain:");
            rep(
                "music-user-token",
                match auth::get() {
                    None => Err("not stored — run `melbad auth open`".into()),
                    Some(t) => musickit::validate_token(&t).map(|_| "VALID".to_string()),
                },
            );
            rep(
                "developer-token",
                musickit::dev_token().map(|_| "ok".to_string()),
            );
            rep(
                "widevine cdm",
                stream::cdm_so()
                    .map(|p| p.display().to_string())
                    .ok_or_else(|| {
                        "not found — install Chrome/Chromium/Vivaldi/Brave or \
                         set cdm_path in config.json"
                            .into()
                    }),
            );
            rep(
                "alac wrapper",
                if melbad::wrapper::managed() {
                    match melbad::wrapper::installed() {
                        None => {
                            Err("managed mode but not installed — `melbad wrapper setup`".into())
                        }
                        Some(b) => {
                            match melbad::wrapper::check_wrapper(&melbad::wrapper::key_url()) {
                                Ok(s) => Ok(format!("managed {s}")),
                                Err(_) => Ok(format!(
                                    "installed {} (starts on first ALAC play)",
                                    b.display()
                                )),
                            }
                        }
                    }
                } else {
                    match melbad::config::wrapper_url() {
                        None => match melbad::config::wrapper_url_rejection() {
                            Some(e) => Err(e),
                            None => Ok("not configured (AAC-256 only)".into()),
                        },
                        Some(u) => melbad::wrapper::check_wrapper(&u),
                    }
                },
            );
            rep(
                "audio output",
                if std::path::Path::new("/dev/snd").exists() {
                    Ok("alsa present".into())
                } else {
                    Err("/dev/snd missing".into())
                },
            );
            if bad > 0 {
                std::process::exit(1);
            }
        }
        Cmd::Wrapper { op } => match op {
            WrapOp::Setup { apk } => {
                match melbad::wrapper::setup(apk.as_deref().map(std::path::Path::new)) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("wrapper setup: {e}");
                        std::process::exit(1);
                    }
                }
            }
            WrapOp::Login { user, pass } => {
                if pass.is_some() {
                    eprintln!(
                        "wrapper login: the password is not accepted as an argument. \
                         Type it at the prompt, or set MELBA_WRAPPER_PASSWORD."
                    );
                    std::process::exit(2);
                }
                let pass = match melbad::wrapper::read_login_password(&user) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("wrapper login: {e}");
                        std::process::exit(1);
                    }
                };
                match melbad::wrapper::login(&user, &pass) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("wrapper login: {e}");
                        std::process::exit(1);
                    }
                }
            }
            WrapOp::Start => match melbad::wrapper::ensure() {
                Ok(u) => println!("listening: {u}"),
                Err(e) => {
                    eprintln!("wrapper start: {e}");
                    std::process::exit(1);
                }
            },
            WrapOp::Stop => {
                if let Err(e) = melbad::wrapper::stop() {
                    eprintln!("wrapper stop: {e}");
                    std::process::exit(1);
                }
            }
            WrapOp::Status => println!("{}", melbad::wrapper::status()),
        },
        Cmd::Mk { op } => {
            let r =
                match op {
                    MkOp::Token => musickit::dev_token(),
                    MkOp::Search {
                        term,
                        types,
                        limit,
                        collection,
                    } => {
                        if collection {
                            musickit::collection_from(&term, limit)
                        } else {
                            musickit::search(&term, &types, limit)
                                .map(|v| serde_json::to_string_pretty(&v).unwrap())
                        }
                    }
                    MkOp::Collection { kind, id } => musickit::collection_of(&kind, &id),
                    MkOp::Album { id } => musickit::lookup_album(&id)
                        .map(|v| serde_json::to_string_pretty(&v).unwrap()),
                    MkOp::Artist { id } => {
                        musickit::artist(&id).map(|v| serde_json::to_string_pretty(&v).unwrap())
                    }
                    MkOp::Song { id } => musickit::lookup_song(&id)
                        .map(|v| serde_json::to_string_pretty(&v).unwrap()),
                    MkOp::Me { what, limit } => musickit::me(&what, limit)
                        .map(|v| serde_json::to_string_pretty(&v).unwrap()),
                    MkOp::Add { rtype, id } => {
                        musickit::library_add(&rtype, &id).map(|_| "added".to_string())
                    }
                    MkOp::Lyrics { id } => {
                        musickit::lyrics(&id).map(|v| serde_json::to_string_pretty(&v).unwrap())
                    }
                    MkOp::Enrich { file, limit } => {
                        let text = if file == "-" {
                            use std::io::Read;
                            let mut b = String::new();
                            if let Err(e) = std::io::stdin().read_to_string(&mut b) {
                                eprintln!("mk: stdin: {e}");
                                std::process::exit(11);
                            }
                            b
                        } else {
                            match std::fs::read_to_string(&file) {
                                Ok(t) => t,
                                Err(e) => {
                                    eprintln!("mk: {file}: {e}");
                                    std::process::exit(11);
                                }
                            }
                        };
                        musickit::enrich_collection(&text, limit)
                    }
                };
            match r {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("mk: {e}");
                    std::process::exit(11);
                }
            }
        }
        Cmd::Import { path, schema } | Cmd::Scan { dir: path, schema } => {
            let p = Path::new(&path);
            let c = if p.is_dir() {
                library::scan_dir(p)
            } else {
                library::import(p)
            };
            match c.and_then(|c| library::to_collection_json(&c)) {
                Ok(j) => {
                    if let Some(sp) = schema {
                        match library::validate_against_schema(Path::new(&sp), &j) {
                            Ok(()) => eprintln!("schema: VALID"),
                            Err(e) => {
                                eprintln!("schema: INVALID — {e}");
                                std::process::exit(8);
                            }
                        }
                    }
                    println!("{j}");
                }
                Err(e) => {
                    eprintln!("import failed: {e}");
                    std::process::exit(2);
                }
            }
        }
        Cmd::Config { op } => match op {
            ConfigOp::AudioDevice { value } => match value {
                Some(value) => match melbad::config::set_audio_device(&value) {
                    Ok(()) => println!("audio_device = {value}"),
                    Err(e) => {
                        eprintln!("config audio-device: {e}");
                        std::process::exit(2);
                    }
                },
                None => println!(
                    "audio_device = {}",
                    melbad::config::audio_device().unwrap_or_else(|| "auto (unset)".into())
                ),
            },
        },
        Cmd::Setup { yes } => {
            let devices: Vec<_> = audio::list_devices()
                .into_iter()
                .filter(|d| d.usable() && d.device.starts_with("hw:"))
                .collect();
            println!("Audio outputs:");
            println!("  0  auto (prefer USB/analog, never the OS mixer)");
            println!("  1  default (OS mixer; not bit-perfect)");
            for (i, d) in devices.iter().enumerate() {
                println!(
                    "  {}  {} ({})  {}..{} Hz, {}..{} ch",
                    i + 2,
                    d.card_name,
                    d.device,
                    d.rate_min,
                    d.rate_max,
                    d.ch_min,
                    d.ch_max
                );
            }
            let choice = if yes {
                "auto".to_string()
            } else {
                println!("Select [0]: ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                match input.trim() {
                    "" | "0" => "auto".to_string(),
                    "1" => "default".to_string(),
                    n => n
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| devices.get(n.saturating_sub(2)))
                        .map(|d| d.device.clone())
                        .unwrap_or_else(|| "auto".to_string()),
                }
            };
            if let Err(e) = melbad::config::set_audio_device(&choice) {
                eprintln!("setup: {e}");
                std::process::exit(2);
            }
            println!("audio_device = {choice}");
            let wrapper = melbad::config::wrapper_url();
            match wrapper {
                Some(url) => println!("wrapper: configured ({url}); sign in in the GUI to use it"),
                None if melbad::wrapper::installed().is_some() => println!(
                    "wrapper: managed service installed; Melba starts it automatically for ALAC"
                ),
                None => println!(
                    "wrapper: no external service configured; ALAC stays unavailable. \
Sign in in the GUI for AAC, or run your own service and set wrapper_url."
                ),
            }
            println!("Next: start `melba`, sign in, and use the Settings output picker to verify.");
            println!("This command never downloads, installs or bundles a wrapper.");
        }
        Cmd::Play {
            file,
            device,
            require_bitperfect,
            verify,
            max_seconds,
        } => {
            let device = resolve_device(device);
            if !refuse_shared(&device, require_bitperfect) {
                std::process::exit(2);
            }
            std::process::exit(run_play(&file, &device, verify, max_seconds))
        }
    }
}

fn now_json() -> Result<(), String> {
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;
    use std::time::Duration;
    let conn = Connection::new_session().map_err(|e| format!("dbus: {e}"))?;
    let p = conn.with_proxy(
        "org.mpris.MediaPlayer2.melba",
        "/org/mpris/MediaPlayer2",
        Duration::from_secs(2),
    );
    let iface = "org.mpris.MediaPlayer2.Player";
    let meta = p
        .get::<dbus::arg::PropMap>(iface, "Metadata")
        .unwrap_or_default();
    let get_s = |k: &str| {
        meta.get(k)
            .and_then(|v| v.0.as_str())
            .map(|s| s.to_string())
    };
    let get_i = |k: &str| meta.get(k).and_then(|v| v.0.as_i64()).unwrap_or(0);
    let get_u = |k: &str| meta.get(k).and_then(|v| v.0.as_u64()).unwrap_or(0);
    let queue: Vec<String> = meta
        .get("melba:queue")
        .and_then(|v| v.0.as_iter())
        .map(|it| {
            it.filter_map(|a| a.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| v.0.as_iter())
        .map(|it| {
            it.filter_map(|a| a.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let out = serde_json::json!({
        "status": p.get::<String>(iface, "PlaybackStatus").unwrap_or_else(|_| "Stopped".into()),
        "positionMs": p.get::<i64>(iface, "Position").unwrap_or(0) / 1000,
        "durationMs": get_i("mpris:length") / 1000,
        "title": get_s("xesam:title"),
        "artist": artist,
        "album": get_s("xesam:album"),
        "artUrl": get_s("mpris:artUrl"),
        "uri": get_s("xesam:url"),
        "queueIndex": get_u("melba:idx"),
        "queue": queue,
        "shuffle": p.get::<bool>(iface, "Shuffle").unwrap_or(false),
        "loopStatus": p.get::<String>(iface, "LoopStatus").unwrap_or_else(|_| "None".into()),
        "sink": get_s("melba:sink"),
        "device": get_s("melba:device"),
        "verdict": get_s("melba:verdict"),
        "codec": get_s("melba:codec"),
        "sampleRate": get_u("melba:sampleRate"),
        "bitDepth": get_u("melba:bitDepth"),
        "channels": get_u("melba:channels"),
        "volume": p.get::<f64>(iface, "Volume").unwrap_or(1.0),
        "streamOpens": get_u("melba:streamOpens"),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    Ok(())
}

fn about_manifest() -> serde_json::Value {
    serde_json::json!({
        "name": "melbad",
        "version": env!("CARGO_PKG_VERSION"),
        "what": "native Linux Apple Music-shaped client: bit-perfect local hi-res + catalog metadata + full-track subscription streaming (AAC-256 via system Widevine CDM; ALAC via optional local decrypt service)",
        "commands": {
            "probe": "list usable hw: PCM devices + capability bounds (--all shows failed slots too)",
            "play": "melbad play <file> [--verify] [--require-bitperfect] [--device auto|default|hw:X,Y] — receipt JSON with verdict BITPERFECT|CLAMPED|MISMATCH",
            "serve": "daemon + MPRIS org.mpris.MediaPlayer2.melba over files or a sone.collection/v1 json (playUri may be preview:[cc:]<apple-id> or stream:<apple-id> for subscription full tracks); honours audio_device from config; no args restores the last persisted queue",
            "ctl": "transport: play pause playpause stop next previous status seek setpos loop shuffle volume open enqueue load reorder goto clear device quit",
            "doctor": "one-shot diagnosis: token (live), dev token, CDM, wrapper, audio — exit non-zero on failure",
            "now": "full state as JSON",
            "import|scan": "playlists/dir -> sone.collection/v1 (+--schema validate)",
            "mk": "token | search [--collection] | collection <albums|playlists> <id> | song | album | artist | enrich | me <what> | add <type> <id> | lyrics <id>",
            "auth": "set|status|clear|how|open[--browser] — music-user-token (0600)",
            "preview": "melbad preview <song-id> — download 30s AAC and play through hw:",
            "dl": "melbad dl <song-id> — preview+artwork+manifest into ~/.cache/melba/downloads/",
        },
        "machine_interfaces": {
            "mpris": "org.mpris.MediaPlayer2.melba",
            "mpris_player": "Play Pause PlayPause Stop Next Previous Seek SetPosition OpenUri(file://,preview:[cc:]<id>,stream:<id>) + MelbaLoadQueue(as,u,u,b) + MelbaGoTo + MelbaQueueReorder(u,u) + MelbaQueueClear + MelbaEnqueue(as) + MelbaSetDevice(s); props incl. xesam:{title,artist,album,url} mpris:{trackid,length,artUrl} melba:{queue,idx,titles,artists,artUrls,verdict,sink,device,volume}; props Volume; signals: PropertiesChanged, Seeked",
            "stdout": "JSON everywhere it matters",
            "collection_contract": "sone.collection/v1 (contracts/sone.collection.v1.schema.json)",
        },
        "boundaries": [
            "full-track streaming uses your own subscription token + the system Widevine CDM (crates/melba-cdm): webPlayback ctrp flavor, in-process segment decrypt, verified end-to-end",
            "ALAC/lossless: enhancedHls is fetched and parsed; track keys need a user-supplied local decrypt service (config.json wrapper_url: tcp://host:port = wrapper-v2, http(s)://… = key extraction)",
            "no Apple credentials are handled; user token is user-supplied, stored 0600",
        ],
    })
}
