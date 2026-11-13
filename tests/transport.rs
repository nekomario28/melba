//! Exercise the daemon's real D-Bus receiver and decoder without an account or hardware.
use dbus::blocking::{stdintf::org_freedesktop_dbus::Properties, Connection};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const BUS: &str = "org.mpris.MediaPlayer2.melba";
const IFACE: &str = "org.mpris.MediaPlayer2.Player";

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Debug)]
struct Snapshot {
    queue: Vec<String>,
    index: u64,
    uri: String,
    status: String,
    rate: u64,
    position: i64,
    track_id: String,
    /// streams opened this session — 0 means no device has been taken
    stream_opens: u64,
    next_indices: Option<Vec<u64>>,
}
fn snapshot(conn: &Connection) -> Result<Snapshot, dbus::Error> {
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_millis(200));
    let meta: dbus::arg::PropMap = proxy.get(IFACE, "Metadata")?;
    Ok(Snapshot {
        queue: meta
            .get("melba:queue")
            .and_then(|v| v.0.as_iter())
            .map(|it| it.filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        index: meta
            .get("melba:idx")
            .and_then(|v| v.0.as_u64())
            .unwrap_or_else(|| {
                panic!(
                    "missing queue cursor; metadata keys: {:?}",
                    meta.keys().collect::<Vec<_>>()
                )
            }),
        uri: meta
            .get("xesam:url")
            .and_then(|v| v.0.as_str())
            .unwrap_or_default()
            .into(),
        rate: meta
            .get("melba:sampleRate")
            .and_then(|v| v.0.as_u64())
            .unwrap_or(0),
        status: proxy.get(IFACE, "PlaybackStatus")?,
        position: proxy.get::<i64>(IFACE, "Position")? / 1000,
        track_id: meta
            .get("mpris:trackid")
            .and_then(|v| v.0.as_str())
            .unwrap_or_default()
            .into(),
        stream_opens: meta
            .get("melba:streamOpens")
            .and_then(|v| v.0.as_u64())
            .unwrap_or(0),
        next_indices: meta
            .get("melba:nextIndices")
            .and_then(|v| v.0.as_iter())
            .map(|it| it.filter_map(|v| v.as_u64()).collect()),
    })
}
#[track_caller]
fn wait(conn: &Connection, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        match snapshot(conn) {
            Ok(s) if predicate(&s) => return s,
            other if Instant::now() >= deadline => {
                panic!("daemon did not reach expected state: {other:?}")
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn private_case(name: &str) -> bool {
    const CHILD: &str = "MELBA_TRANSPORT_PRIVATE_BUS";
    if std::env::var_os(CHILD).is_none() {
        let root = std::env::temp_dir().join(format!(
            "melba-ctl-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        for dir in ["config", "cache", "data", "state", "run"] {
            std::fs::create_dir(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("alsa.conf"), "pcm.null { type null }\n").unwrap();
        let output = Command::new("dbus-run-session")
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, "1")
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .env("ALSA_CONFIG_PATH", root.join("alsa.conf"))
            .env(
                "MELBA_TRANSPORT_PARENT_BUS",
                std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default(),
            )
            .env(
                "DBUS_SYSTEM_BUS_ADDRESS",
                "unix:path=/nonexistent/melba-transport-system-bus",
            )
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .output()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
        return false;
    }
    assert_ne!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
        std::env::var("MELBA_TRANSPORT_PARENT_BUS").unwrap()
    );
    true
}

/// Shuffle must retrace played occurrences, not subtract a raw queue index
/// or draw a new random song when going forward after Previous.
#[test]
fn shuffled_previous_and_next_retrace_the_played_occurrence() {
    if !private_case("shuffled_previous_and_next_retrace_the_played_occurrence") {
        return;
    }
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["serve", "--device", "null"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let conn = Connection::new_session().unwrap();
    wait(&conn, |s| s.status == "Stopped");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let queue: Vec<String> = ["44k-16bit-art.m4a", "96k-24bit.m4a", "48k-mono-aac.m4a"]
        .iter()
        .map(|name| format!("file://{}", fixtures.join(name).display()))
        .collect();
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(2));
    proxy.set(IFACE, "Shuffle", true).unwrap();
    proxy
        .method_call::<(), _, _, _>(IFACE, "MelbaLoadQueue", (queue.clone(), 2u32, 0u32, false))
        .unwrap();
    let original = wait(&conn, |s| {
        s.status == "Paused" && s.index == 2 && s.rate == 48000
    });
    proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
    let skipped = wait(&conn, |s| {
        s.status == "Paused" && s.index != 2 && s.track_id != original.track_id
    });
    assert_eq!(
        original
            .next_indices
            .as_ref()
            .expect("daemon must publish its actual plan")[0],
        skipped.index
    );
    // Starting at the last raw row makes the old index-1 implementation
    // wrong for every possible shuffled choice, not merely a lucky seed.
    let mut previous_id = skipped.track_id;
    for _ in 0..4 {
        proxy
            .method_call::<(), _, _, _>(IFACE, "Previous", ())
            .unwrap();
        let back = wait(&conn, |s| s.status == "Paused" && s.track_id != previous_id);
        assert_eq!(
            back.index, 2,
            "Previous must return to the occurrence actually played"
        );
        assert_eq!(back.uri, queue[2]);
        assert_eq!(
            back.next_indices
                .as_ref()
                .expect("back must retain its forward plan")[0],
            skipped.index
        );
        proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
        let forward = wait(&conn, |s| {
            s.status == "Paused" && s.track_id != back.track_id
        });
        assert_eq!(
            forward.index, skipped.index,
            "Next after Previous must reuse the visited occurrence, not reroll"
        );
        previous_id = forward.track_id;
    }
    // A manual jump also has a real predecessor. Returning to a last raw row
    // must not disable Next while the forward history points at an earlier row.
    proxy.set(IFACE, "Shuffle", false).unwrap();
    proxy
        .method_call::<(), _, _, _>(IFACE, "MelbaLoadQueue", (queue.clone(), 2u32, 0u32, false))
        .unwrap();
    wait(&conn, |s| {
        s.status == "Paused" && s.index == 2 && s.rate == 48000
    });
    // GoTo intentionally starts playback. The unpaced null sink must be
    // held on that row until Pause, rather than assuming selection is paused.
    proxy.set(IFACE, "LoopStatus", "Track".to_owned()).unwrap();
    proxy
        .method_call::<(), _, _, _>(IFACE, "MelbaGoTo", (0u32,))
        .unwrap();
    wait(&conn, |s| {
        s.status == "Playing" && s.index == 0 && s.rate == 44100
    });
    proxy
        .method_call::<(), _, _, _>(IFACE, "Pause", ())
        .unwrap();
    wait(&conn, |s| s.status == "Paused" && s.index == 0);
    proxy.set(IFACE, "LoopStatus", "None".to_owned()).unwrap();
    proxy
        .method_call::<(), _, _, _>(IFACE, "Previous", ())
        .unwrap();
    wait(&conn, |s| {
        s.status == "Paused" && s.index == 2 && s.rate == 48000
    });
    assert!(
        proxy.get::<bool>(IFACE, "CanGoNext").unwrap(),
        "a retained forward occurrence must enable Next"
    );
    proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
    wait(&conn, |s| {
        s.status == "Paused" && s.index == 0 && s.rate == 44100
    });
}

#[test]
fn previous_skips_sources_that_never_opened() {
    if !private_case("previous_skips_sources_that_never_opened") {
        return;
    }
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["serve", "--device", "null"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let conn = Connection::new_session().unwrap();
    wait(&conn, |s| s.status == "Stopped");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let queue = vec![
        format!("file://{}", fixtures.join("44k-16bit-art.m4a").display()),
        format!("file://{}/missing.m4a", std::env::var("HOME").unwrap()),
        format!("file://{}", fixtures.join("48k-mono-aac.m4a").display()),
    ];
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(2));
    proxy
        .method_call::<(), _, _, _>(IFACE, "MelbaLoadQueue", (queue, 0u32, 0u32, false))
        .unwrap();
    wait(&conn, |s| {
        s.status == "Paused" && s.index == 0 && s.rate == 44100
    });
    proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
    let skipped = wait(&conn, |s| {
        s.status == "Paused" && s.index == 2 && s.rate == 48000
    });
    proxy
        .method_call::<(), _, _, _>(IFACE, "Previous", ())
        .unwrap();
    let back = wait(&conn, |s| {
        s.status == "Paused" && s.track_id != skipped.track_id
    });
    assert_eq!(
        back.index, 0,
        "failed sources must not enter played history"
    );
}

#[test]
fn queue_load_preserves_the_context_selection_and_paused_skips() {
    if !private_case("queue_load_preserves_the_context_selection_and_paused_skips") {
        return;
    }
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["serve", "--device", "null"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let conn = Connection::new_session().unwrap();
    wait(&conn, |s| s.status == "Stopped");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let queue: Vec<String> = ["44k-16bit-art.m4a", "96k-24bit.m4a", "48k-mono-aac.m4a"]
        .iter()
        .map(|f| format!("file://{}", fixtures.join(f).display()))
        .collect();
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(2));
    proxy
        .method_call::<(), _, _, _>(IFACE, "MelbaLoadQueue", (queue.clone(), 1u32, 0u32, false))
        .unwrap();
    let selected = wait(&conn, |s| {
        s.status == "Paused" && s.uri == queue[1] && s.rate == 96000
    });
    let paths: Vec<String> = queue
        .iter()
        .map(|uri| uri.strip_prefix("file://").unwrap().to_owned())
        .collect();
    assert_eq!(selected.queue, paths);
    assert_eq!(selected.index, 1);
    proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
    let next = wait(&conn, |s| {
        s.status == "Paused" && s.uri == queue[2] && s.rate == 48000
    });
    assert_eq!(next.queue, paths);
    assert_eq!(next.index, 2);
    proxy
        .method_call::<(), _, _, _>(IFACE, "Previous", ())
        .unwrap();
    wait(&conn, |s| {
        s.status == "Paused" && s.uri == queue[1] && s.rate == 96000
    });
    let shorter = vec![queue[0].clone(), queue[1].clone(), queue[0].clone()];
    proxy
        .method_call::<(), _, _, _>(
            IFACE,
            "MelbaLoadQueue",
            (shorter.clone(), 2u32, 120u32, false),
        )
        .unwrap();
    let replaced = wait(&conn, |s| {
        s.status == "Paused" && s.uri == shorter[2] && s.position == 120
    });
    let shorter_paths: Vec<String> = shorter
        .iter()
        .map(|uri| uri.strip_prefix("file://").unwrap().to_owned())
        .collect();
    assert_eq!(replaced.queue, shorter_paths);
    assert_eq!(replaced.index, 2);
    assert!(proxy
        .method_call::<(), _, _, _>(
            IFACE,
            "MelbaLoadQueue",
            (shorter.clone(), 99u32, 0u32, true)
        )
        .is_err());
    assert!(proxy
        .method_call::<(), _, _, _>(
            IFACE,
            "MelbaLoadQueue",
            (
                vec!["https://invalid.example/not-audio".to_string()],
                0u32,
                0u32,
                true
            )
        )
        .is_err());
    assert_eq!(
        snapshot(&conn).unwrap().queue,
        shorter_paths,
        "invalid loads must not partially change the queue"
    );
    // Load also works from Stop's waiting loop, not only from paused playback.
    proxy.method_call::<(), _, _, _>(IFACE, "Stop", ()).unwrap();
    wait(&conn, |s| s.status == "Stopped" && s.index == 2);
    // Pausing must carry across valid skips, not leak into a later append
    // after Next has exhausted the context.
    for method in ["OpenUri", "MelbaEnqueue"] {
        proxy
            .method_call::<(), _, _, _>(
                IFACE,
                "MelbaLoadQueue",
                (vec![queue[0].clone()], 0u32, 0u32, false),
            )
            .unwrap();
        wait(&conn, |s| {
            s.status == "Paused" && s.index == 0 && s.queue.len() == 1
        });
        proxy.method_call::<(), _, _, _>(IFACE, "Next", ()).unwrap();
        wait(&conn, |s| s.status == "Stopped" && s.index == 1);
        if method == "OpenUri" {
            proxy
                .method_call::<(), _, _, _>(IFACE, method, (queue[0].clone(),))
                .unwrap();
        } else {
            proxy
                .method_call::<(), _, _, _>(IFACE, method, (vec![queue[0].clone()],))
                .unwrap();
        }
        let ended = wait(&conn, |s| {
            s.status == "Stopped" && s.index == 2 && s.queue.len() == 2
        });
        assert_eq!(
            ended.queue,
            vec![paths[0].clone(), paths[0].clone()],
            "{method} must start playback, not inherit a previous exhausted queue's pause"
        );
    }
    // Drive the public CLI too. Choosing the last row must prepare exactly
    // one decoder, not start at the first row and eventually reach the last.
    let sequence = || {
        let meta: dbus::arg::PropMap = proxy.get(IFACE, "Metadata").unwrap();
        meta["mpris:trackid"]
            .0
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    let before = sequence();
    let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args(["ctl", "load", "1", &queue[0], &queue[1]])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let loaded = wait(&conn, |s| {
        s.status == "Stopped" && s.index == 2 && s.queue == paths[..2]
    });
    assert_eq!(loaded.queue, paths[..2]);
    assert_eq!(
        sequence(),
        before + 1,
        "CLI load must start at its selected row"
    );
}

#[test]
fn desktop_startup_is_paused_and_mpris_actions_reach_the_frontend() {
    if !private_case("desktop_startup_is_paused_and_mpris_actions_reach_the_frontend") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let bin = PathBuf::from(std::env::var_os("HOME").unwrap()).join("bin");
    std::fs::create_dir(&bin).unwrap();
    let daemon_bin = bin.join("melbad");
    if std::fs::hard_link(env!("CARGO_BIN_EXE_melbad"), &daemon_bin).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_melbad"), &daemon_bin).unwrap();
    }
    let log = bin.join("forwarded");
    let frontend = bin.join("melba");
    std::fs::write(
        &frontend,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$MELBA_TEST_FORWARD_LOG\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&frontend, std::fs::Permissions::from_mode(0o700)).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/96k-24bit.m4a");
    let mut daemon = Daemon(
        Command::new(&daemon_bin)
            .args(["serve", "--device", "null", "--paused"])
            .arg(&fixture)
            .env("MELBA_TEST_FORWARD_LOG", &log)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let connection = Connection::new_session().unwrap();
    let initial = wait(&connection, |s| s.status == "Paused" && s.rate == 96000);
    assert_eq!(initial.queue, [fixture.to_string_lossy()]);
    assert_eq!(initial.index, 0);
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        snapshot(&connection).unwrap().position,
        initial.position,
        "opening the desktop must not autoplay a restored queue"
    );
    // A second server must fail instead of stealing/queueing the control name.
    let mut second = Daemon(
        Command::new(&daemon_bin)
            .args(["serve", "--device", "null"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(second.0.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!second
        .0
        .try_wait()
        .unwrap()
        .expect("duplicate server must exit")
        .success());
    let proxy = connection.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(3));
    assert!(proxy
        .get::<bool>("org.mpris.MediaPlayer2", "CanRaise")
        .unwrap());
    proxy
        .method_call::<(), _, _, _>("org.mpris.MediaPlayer2", "Raise", ())
        .unwrap();
    proxy
        .method_call::<(), _, _, _>(IFACE, "OpenUri", ("melba:album:fixture",))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "show\nplay-uri\nmelba:album:fixture\n"
    );
    let after = snapshot(&connection).unwrap();
    assert_eq!(after.queue, initial.queue);
    assert_eq!(after.index, initial.index);
    assert_eq!(
        after.status, "Paused",
        "forwarding a context link is not a local append"
    );
    assert!(proxy
        .method_call::<(), _, _, _>(IFACE, "OpenUri", ("unknown:fixture",))
        .is_err());
    proxy
        .method_call::<(), _, _, _>("org.mpris.MediaPlayer2", "Quit", ())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(daemon.0.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(daemon
        .0
        .try_wait()
        .unwrap()
        .expect("Quit must stop the process, not only its bus facade")
        .success());
}

/// A desktop session starts paused. It must not take the output device to say
/// so: opening an exclusive PCM and pausing it reserves the card for as long as
/// the window is open (measured on hw:1,7 — `fuser` listed the daemon and
/// nothing else could use the card).
#[test]
fn paused_startup_takes_no_output_device() {
    if !private_case("paused_startup_takes_no_output_device") {
        return;
    }
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["serve", "--device", "null"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let conn = Connection::new_session().unwrap();
    wait(&conn, |s| s.status == "Stopped");
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(2));
    proxy
        .method_call::<(), _, _, _>(
            IFACE,
            "MelbaLoadQueue",
            (
                vec![format!(
                    "file://{}",
                    fixtures.join("44k-16bit-art.m4a").display()
                )],
                0u32,
                0u32,
                false,
            ),
        )
        .unwrap();
    // The selection is known, and described, without a device.
    let paused = wait(&conn, |s| {
        s.status == "Paused" && s.index == 0 && s.rate == 44100
    });
    assert_eq!(
        paused.stream_opens, 0,
        "a paused player must not hold the output device"
    );
    proxy.method_call::<(), _, _, _>(IFACE, "Play", ()).unwrap();
    let playing = wait(&conn, |s| s.status == "Playing" && s.stream_opens > 0);
    assert_eq!(playing.index, 0, "Play resumes the selected entry");
}

/// Exercise the real reopen loop with deterministic pause/resume failures.
/// The shim throttles ALSA null, so position assertions cannot race through EOF.
fn pause_case(name: &str, failure: &str, seek_while_parked: bool) {
    if !private_case(name) {
        return;
    }
    let root = PathBuf::from(std::env::var_os("XDG_STATE_HOME").unwrap());
    let shim = root.join("pcm_pause.so");
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Werror"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pcm_pause.c"))
        .args(["-ldl", "-o"])
        .arg(&shim)
        .status()
        .unwrap()
        .success());
    // 45 seconds of stereo 16-bit silence at 48 kHz.
    let bytes = 45u32 * 48000 * 4;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    wav.extend_from_slice(&48000u32.to_le_bytes());
    wav.extend_from_slice(&192000u32.to_le_bytes());
    wav.extend_from_slice(b"\x04\0\x10\0data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    wav.resize(44 + bytes as usize, 0);
    let file = root.join("pause.wav");
    std::fs::write(&file, wav).unwrap();
    let trace = root.join("pause.trace");
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_melbad"))
            .args(["serve", "--device", "null", "--paused"])
            .arg(file)
            .env("LD_PRELOAD", shim)
            .env("MELBA_TEST_PCM_THROTTLE", "1")
            // Preserve a 200 ms tail even if failed Resume invalidates delay.
            .env(
                "MELBA_TEST_PAUSED_QUEUE_FRAMES",
                match failure {
                    "resume" => "9600",
                    "unknown" => "-1",
                    _ => "0",
                },
            )
            .env("MELBA_TEST_PAUSE_FAILURE", failure)
            .env("MELBA_TEST_PAUSE_TRACE", &trace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let conn = Connection::new_session().unwrap();
    let proxy = conn.with_proxy(BUS, "/org/mpris/MediaPlayer2", Duration::from_secs(2));
    wait(&conn, |s| s.status == "Paused" && s.rate == 48000);
    proxy.method_call::<(), _, _, _>(IFACE, "Play", ()).unwrap();
    wait(&conn, |s| s.status == "Playing" && s.position >= 500);
    for cycle in 0..2 {
        proxy
            .method_call::<(), _, _, _>(IFACE, "Pause", ())
            .unwrap();
        let paused = wait(&conn, |s| s.status == "Paused");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(snapshot(&conn).unwrap().position, paused.position);
        let target = if seek_while_parked {
            15000 + cycle * 5000
        } else {
            paused.position - 200
        };
        if seek_while_parked {
            proxy
                .method_call::<(), _, _, _>(
                    IFACE,
                    "SetPosition",
                    (
                        dbus::Path::new(paused.track_id.clone()).unwrap(),
                        target * 1000,
                    ),
                )
                .unwrap();
            wait(&conn, |s| s.position == target);
        }
        proxy.method_call::<(), _, _, _>(IFACE, "Play", ()).unwrap();
        if failure == "unknown" {
            wait(&conn, |s| s.status == "Stopped");
            let output = Command::new(env!("CARGO_BIN_EXE_melbad"))
                .arg("now")
                .output()
                .unwrap();
            let state: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                state["verdict"].as_str().unwrap().contains("paused avail"),
                "{state}"
            );
            assert_eq!(
                std::fs::read_to_string(trace)
                    .unwrap()
                    .lines()
                    .filter(|line| *line == "reject 0")
                    .count(),
                1
            );
            return;
        }
        let resumed = wait(&conn, |s| {
            s.status == "Playing" && s.stream_opens > paused.stream_opens
        });
        assert!(
            (target..target + if failure == "resume" { 100 } else { 500 })
                .contains(&resumed.position),
            "resume lost cursor: target={target}, got {resumed:?}"
        );
        wait(&conn, |s| s.position >= target + 500);
    }
    if seek_while_parked && failure != "seek" {
        proxy
            .method_call::<(), _, _, _>(IFACE, "Pause", ())
            .unwrap();
        wait(&conn, |s| s.status == "Paused");
        proxy.method_call::<(), _, _, _>(IFACE, "Stop", ()).unwrap();
        let stopped = wait(&conn, |s| s.status == "Stopped" && s.position == 0);
        proxy.method_call::<(), _, _, _>(IFACE, "Play", ()).unwrap();
        let restarted = wait(&conn, |s| {
            s.status == "Playing" && s.stream_opens > stopped.stream_opens
        });
        assert!(
            restarted.position < 500,
            "Stop must discard the parked cursor: {restarted:?}"
        );
    }
    let trace = std::fs::read_to_string(trace).unwrap();
    if failure == "resume" {
        assert_eq!(
            trace.lines().filter(|line| *line == "queued 9600").count(),
            2,
            "capture the paused buffer before each failed Resume: {trace}"
        );
    }
    let expected = if failure == "pause" {
        "reject 1"
    } else {
        "reject 0"
    };
    assert_eq!(
        trace.lines().filter(|line| *line == expected).count(),
        match failure {
            "seek" => 0,
            "pause" => 3,
            _ => 2,
        },
        "fault path must execute only when expected: {trace}"
    );
}

#[test]
fn unsupported_pause_reopens_at_the_parked_seek() {
    pause_case(
        "unsupported_pause_reopens_at_the_parked_seek",
        "pause",
        true,
    );
}

#[test]
fn seek_while_natively_paused_discards_the_old_buffer() {
    pause_case(
        "seek_while_natively_paused_discards_the_old_buffer",
        "seek",
        true,
    );
}

#[test]
fn failed_native_resume_with_unknown_buffer_stops_instead_of_skipping() {
    pause_case(
        "failed_native_resume_with_unknown_buffer_stops_instead_of_skipping",
        "unknown",
        false,
    );
}

#[test]
fn failed_native_resume_reopens_and_progresses() {
    pause_case(
        "failed_native_resume_reopens_and_progresses",
        "resume",
        false,
    );
}
