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
