//! Protocol tests use private buses, homes and a bounded loopback feed. No
//! desktop input, real account, service-manager commands or audio device.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn private_case(name: &str) -> bool {
    if std::env::var("MELBA_DESKTOP_CASE").as_deref() == Ok(name) {
        assert_ne!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            std::env::var("MELBA_DESKTOP_PARENT_BUS").unwrap()
        );
        return true;
    }
    let root = std::env::temp_dir().join(format!("mlb-ux-{:016x}", rand::random::<u64>()));
    std::fs::create_dir_all(&root).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    for dir in ["config", "state", "cache", "data", "run"] {
        std::fs::create_dir(root.join(dir)).unwrap();
    }
    let output = std::process::Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("MELBA_DESKTOP_CASE", name)
        .env(
            "MELBA_DESKTOP_PARENT_BUS",
            std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default(),
        )
        .env("HOME", &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_RUNTIME_DIR", root.join("run"))
        .env(
            "DBUS_SYSTEM_BUS_ADDRESS",
            "unix:path=/nonexistent-melba-test-bus",
        )
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
    false
}

fn worker(events: Sender<Event>) -> DaemonWorker {
    DaemonWorker::new(
        events,
        crate::backend::Waker::default(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        crate::settings::ProxyConfig::Off,
    )
}

fn fake_daemon() {
    use dbus::channel::{MatchingReceiver, Sender as _};
    let connection = Connection::new_session().unwrap();
    assert_eq!(
        connection.request_name(BUS, false, false, true).unwrap(),
        dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply::PrimaryOwner
    );
    let quitting = Arc::new(AtomicBool::new(false));
    let quit = quitting.clone();
    let mut slow_metadata = 0u32;
    let mut plan_mode = 0u32;
    connection.start_receive(
        dbus::message::MatchRule::new_method_call(),
        Box::new(move |message, connection| {
            let mut reply = message.method_return();
            match message.member().map(|member| member.to_string()).as_deref() {
                Some("Get") => {
                    let (_, property) = message.read2::<String, String>().unwrap();
                    match property.as_str() {
                        "Metadata" => {
                            if slow_metadata > 0 {
                                slow_metadata -= 1;
                                if slow_metadata == 0 {
                                    std::thread::sleep(Duration::from_millis(500));
                                }
                            }
                            let mut metadata = dbus::arg::PropMap::new();
                            metadata.insert("melba:idx".into(), dbus::arg::Variant(Box::new(0u32)));
                            metadata.insert(
                                "mpris:trackid".into(),
                                dbus::arg::Variant(Box::new(dbus::Path::from(format!(
                                    "/org/mpris/MediaPlayer2/melba/Track/{}",
                                    if plan_mode == 2 { 11 } else { 10 }
                                )))),
                            );
                            metadata.insert(
                                "xesam:url".into(),
                                dbus::arg::Variant(Box::new("stream:101".to_owned())),
                            );
                            metadata.insert(
                                "melba:queue".into(),
                                dbus::arg::Variant(Box::new(vec![
                                    "stream:101".to_owned(),
                                    "stream:102".into(),
                                    "stream:101".into(),
                                ])),
                            );
                            metadata.insert(
                                "melba:titles".into(),
                                dbus::arg::Variant(Box::new(vec![
                                    "Synthetic One".to_owned(),
                                    "Synthetic Two".into(),
                                    "Synthetic One".into(),
                                ])),
                            );
                            metadata.insert(
                                "melba:artists".into(),
                                dbus::arg::Variant(Box::new(vec![
                                    "Synthetic Artist".to_owned();
                                    3
                                ])),
                            );
                            if plan_mode != 2 {
                                let next = if plan_mode == 0 {
                                    vec![1u32, 0, 2]
                                } else {
                                    vec![]
                                };
                                metadata.insert(
                                    "melba:nextIndices".into(),
                                    dbus::arg::Variant(Box::new(next)),
                                );
                            }
                            reply = reply.append1(dbus::arg::Variant(metadata));
                        }
                        "PlaybackStatus" => {
                            reply = reply.append1(dbus::arg::Variant("Stopped".to_owned()))
                        }
                        _ => {} // Unused optional properties fail decoding, not a timed-out RPC.
                    }
                }
                Some("ArmSlowMetadata") => slow_metadata = message.read1::<u32>().unwrap(),
                Some("SetPlanMode") => plan_mode = message.read1::<u32>().unwrap(),
                Some("MelbaAudioDevices") => {
                    reply = reply.append1(
                        "{\"current\":\"hw:3,3\",\"devices\":[{\"device\":\"hw:3,3\",\"card_name\":\"USB Audio\",\"rate_min\":44100,\"rate_max\":192000,\"ch_min\":2,\"ch_max\":2,\"openable\":true,\"note\":\"\"}]}",
                    );
                }
                Some("MelbaSetAudioDevice") => {}
                Some("Quit") => quit.store(true, Ordering::Release),
                _ => {}
            }
            connection.send(reply).unwrap();
            true
        }),
    );
    while !quitting.load(Ordering::Acquire) {
        connection.process(Duration::from_millis(10)).unwrap();
    }
}

struct OwnedChild(std::process::Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_fixture(probe: &DaemonWorker) -> OwnedChild {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "daemon_worker::desktop_tests::desktop_shutdown_stops_its_daemon_but_an_isolated_client_does_not", "--nocapture"])
        .env("MELBA_DESKTOP_DAEMON_FIXTURE", "1")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    OwnedChild(
        probe
            .start_daemon_command(command)
            .expect("daemon startup must wait for a reachable owner"),
    )
}

#[test]
fn daemon_plan_crosses_mpris_and_queue_projection_without_rerolling() {
    const NAME: &str = "daemon_worker::desktop_tests::daemon_plan_crosses_mpris_and_queue_projection_without_rerolling";
    if !private_case(NAME) {
        return;
    }
    let (events, received) = std::sync::mpsc::channel();
    let mut probe = worker(events);
    let _daemon = start_fixture(&probe);
    for mode in 0..3u32 {
        probe
            .proxy()
            .unwrap()
            .method_call::<(), _, _, _>(IFACE, "SetPlanMode", (mode,))
            .unwrap();
        assert_eq!(
            probe.poll_daemon(),
            mode != 1,
            "only a new occurrence should change the paused local snapshot"
        );
        assert_eq!(probe.local.track.as_ref().unwrap().uri, "melba:track:101");
        assert_eq!(
            probe.local.track_sequence,
            if mode == 2 { 2 } else { 1 },
            "MPRIS occurrence identity must reach local history even for the same URI"
        );
        probe.handle_api(ApiRequest::Queue { seq: 77 });
        let queue = received
            .try_iter()
            .find_map(|event| match event {
                Event::Api(answer) => match *answer {
                    ApiResponse::Queue {
                        seq,
                        result: Ok(queue),
                    } => {
                        assert_eq!(seq, 77);
                        Some(queue)
                    }
                    _ => None,
                },
                _ => None,
            })
            .expect("queue response must come from the polled daemon metadata");
        let uris: Vec<_> = queue.queue.iter().map(|item| item.uri()).collect();
        if mode == 0 {
            assert_eq!(
                uris,
                ["melba:track:102", "melba:track:101", "melba:track:101"],
                "UI queue must use the daemon plan, preserving duplicate occurrences"
            );
            assert_eq!(queue.queue[0].name(), "Synthetic Two");
        } else if mode == 1 {
            assert!(
                uris.is_empty(),
                "an explicit empty plan must not fall back to raw queue rows"
            );
        } else {
            assert_eq!(
                uris,
                ["melba:track:102", "melba:track:101"],
                "only a missing property uses the older daemon's raw suffix"
            );
        }
    }
}

#[test]
fn audio_outputs_and_selection_cross_the_worker_api() {
    const NAME: &str =
        "daemon_worker::desktop_tests::audio_outputs_and_selection_cross_the_worker_api";
    if !private_case(NAME) {
        return;
    }
    let (events, received) = std::sync::mpsc::channel();
    let mut probe = worker(events);
    let _daemon = start_fixture(&probe);
    probe.handle_api(ApiRequest::AudioDevices);
    let outputs = received
        .try_iter()
        .find_map(|event| match event {
            Event::Api(answer) => match *answer {
                ApiResponse::AudioDevices(Ok(outputs)) => Some(outputs),
                _ => None,
            },
            _ => None,
        })
        .expect("audio devices response");
    assert_eq!(outputs.current, "hw:3,3");
    assert_eq!(outputs.devices.len(), 1);
    assert_eq!(outputs.devices[0].device, "hw:3,3");
    assert_eq!(outputs.devices[0].rate_max, 192_000);
    probe.handle_api(ApiRequest::SetAudioDevice {
        device: "hw:3,3".into(),
    });
    let saved = received.try_iter().any(|event| {
        matches!(
            event,
            Event::Api(answer) if matches!(*answer, ApiResponse::AudioDeviceSet(Ok(())))
        )
    });
    assert!(saved, "selection must reach the daemon");
}

#[test]
fn shutdown_retries_a_busy_existing_daemon() {
    if !private_case("daemon_worker::desktop_tests::shutdown_retries_a_busy_existing_daemon") {
        return;
    }
    let (events, _) = std::sync::mpsc::channel();
    let probe = worker(events);
    let mut daemon = start_fixture(&probe);
    probe
        .proxy()
        .unwrap()
        .method_call::<(), _, _, _>(IFACE, "ArmSlowMetadata", (1u32,))
        .unwrap();
    // No child handle is passed: this is attachment to an existing daemon.
    // The first metadata probe times out, which must not mean "absent".
    probe.stop_daemon(None);
    assert!(
        !probe.daemon_available(),
        "one timed-out probe must not skip Quit"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(daemon.0.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        daemon
            .0
            .try_wait()
            .unwrap()
            .expect("busy daemon must exit")
            .success()
    );
}

#[test]
fn desktop_shutdown_stops_its_daemon_but_an_isolated_client_does_not() {
    const NAME: &str = "daemon_worker::desktop_tests::desktop_shutdown_stops_its_daemon_but_an_isolated_client_does_not";
    if std::env::var_os("MELBA_DESKTOP_DAEMON_FIXTURE").is_some() {
        fake_daemon();
        return;
    }
    if !private_case(NAME) {
        return;
    }
    let (events, _) = std::sync::mpsc::channel();
    let probe = worker(events);
    assert!(!probe.daemon_available());
    let mut daemon = start_fixture(&probe);
    assert!(probe.daemon_available());
    // An existing owner prevents even an invalid second executable from being
    // launched. The same startup path is used by the real desktop worker.
    assert!(
        probe
            .start_daemon_command(std::process::Command::new("/nonexistent-duplicate-daemon"))
            .is_none()
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    for manage in [false, true] {
        let (events, _) = std::sync::mpsc::channel();
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        commands.send(Command::Shutdown).unwrap();
        runtime.block_on(worker(events).run(receiver, false, manage));
        assert_eq!(
            probe.daemon_available(),
            !manage,
            "shutdown must be opt-in for isolated clients, and effective for the desktop"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(daemon.0.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        daemon
            .0
            .try_wait()
            .unwrap()
            .expect("daemon must exit, not merely acknowledge Quit")
            .success()
    );
    runtime.shutdown_timeout(Duration::from_millis(200));
}

struct Feed {
    source: crate::updates::Source,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Feed {
    fn new() -> Self {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let source =
            crate::updates::Source::local(&format!("http://{}", listener.local_addr().unwrap()))
                .unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Ok((mut connection, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                connection
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut header = [0u8; 4096];
                let count = connection.read(&mut header).unwrap();
                let first = String::from_utf8_lossy(&header[..count])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                recorded.lock().unwrap().push(first);
                let body =
                    br#"{"tag_name":"v999.1.0","html_url":"https://example.invalid/release"}"#;
                write!(connection, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                connection.write_all(body).unwrap();
            }
        });
        Self {
            source,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[test]
fn updater_commands_report_real_results_instead_of_a_no_update_stub() {
    const NAME: &str = "daemon_worker::desktop_tests::updater_commands_report_real_results_instead_of_a_no_update_stub";
    if !private_case(NAME) {
        return;
    }
    let feed = Feed::new();
    let direct = crate::updates::updater(&crate::settings::ProxyConfig::Off)
        .unwrap()
        .with_source(feed.source.clone())
        .check()
        .unwrap()
        .unwrap();
    assert_eq!(direct.version, "999.1.0");
    let (events, received) = std::sync::mpsc::channel();
    let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let task = runtime.spawn(worker(events).run(receiver, false, false));
    commands
        .send(Command::CheckForUpdates {
            manual: true,
            source: feed.source.clone(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if let Event::UpdateChecked { manual, result } = event {
            assert!(manual);
            assert_eq!(result.unwrap(), Some(direct.clone()));
            break;
        }
    }
    assert_eq!(
        *feed.requests.lock().unwrap(),
        ["GET /latest.json HTTP/1.1", "GET /latest.json HTTP/1.1"]
    );
    commands.send(Command::InspectUpdate).unwrap();
    loop {
        if let Event::UpdateSupport(result) = received.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert!(
                result.is_err(),
                "an unmarked test executable must not be installable"
            );
            break;
        }
    }
    commands
        .send(Command::DownloadUpdate {
            release: direct,
            source: feed.source.clone(),
        })
        .unwrap();
    loop {
        if let Event::UpdateDownloaded(result) =
            received.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert!(
                result.is_err(),
                "a different-version daemon must block GUI-only replacement"
            );
            break;
        }
    }
    let sample = crate::updates::Prepared::sample(
        crate::updates::Installation {
            executable: std::env::current_exe().unwrap(),
            kind: crate::updates::Kind::Portable,
        },
        env!("CARGO_PKG_VERSION"),
    );
    commands
        .send(Command::InstallUpdate {
            prepared: Box::new(sample),
            arguments: Vec::new(),
        })
        .unwrap();
    loop {
        if let Event::UpdateInstalling(result) =
            received.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert!(
                result.is_err(),
                "sample preparation must never execute an installer"
            );
            break;
        }
    }
    commands.send(Command::Shutdown).unwrap();
    runtime.block_on(task).unwrap();
    runtime.shutdown_timeout(Duration::from_millis(200));
}
