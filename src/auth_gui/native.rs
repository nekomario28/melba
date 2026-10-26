//! Authentication presentation only. The SDK still owns the account protocol.
use melba_auth::{Button, Event, Input, Prompt, PromptKind, MAX_PACKET};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BRIDGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/melba-auth.so"));
const FAILURE: &str = "native sign-in failed";

fn spawn(command: &mut Command) -> Result<crate::wrapper::process::ProcessGroup, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::wrapper::process::ProcessGroup::spawn(command)
}
fn prepare_bridge(directory: &Path) -> Result<PathBuf, String> {
    crate::wrapper::process::stage(
        &directory.join("rootfs/system/lib64"),
        "melba-auth",
        BRIDGE,
        false,
    )
}
fn token_paths(directory: &Path) -> Vec<PathBuf> {
    let base = directory.join("rootfs/data/data/com.apple.android.music/files");
    let mut paths = vec![base.join("MUSIC_TOKEN")];
    if let Ok(relative) = base.strip_prefix("/") {
        paths.push(directory.join("rootfs").join(relative).join("MUSIC_TOKEN"));
    }
    paths
}
fn identity(path: &Path) -> Option<(u64, u64, i64, i64)> {
    let m = path.symlink_metadata().ok()?;
    if !m.is_file() {
        return None;
    }
    Some((m.dev(), m.ino(), m.ctime(), m.ctime_nsec()))
}
fn read_token(path: &Path) -> Option<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.len() > 16_384
        || metadata.mode() & 0o077 != 0
    {
        return None;
    }
    let mut token = String::new();
    Read::by_ref(&mut file)
        .take(16_385)
        .read_to_string(&mut token)
        .ok()?;
    if token.is_empty() || token.len() > 16_384 || token.chars().any(char::is_control) {
        return None;
    }
    Some(token)
}

pub(super) fn matches_current_account(token: &str) -> bool {
    crate::wrapper::installed()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .is_some_and(|d| {
            token_paths(&d)
                .iter()
                .any(|p| read_token(p).is_some_and(|t| t == token))
        })
}

fn verify_payload(directory: &Path) -> Result<(), String> {
    // Existing listeners are never authentication evidence or ours to kill.
    for port in [10020, 20020, 30020, 40020] {
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(100),
        )
        .is_ok()
        {
            return Err(FAILURE.into());
        }
    }
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../packaging/wrapper-auth-abi.json"))
            .map_err(|_| FAILURE)?;
    let family = manifest["families"]
        .as_array()
        .and_then(|families| {
            families
                .iter()
                .find(|family| family["arch"] == std::env::consts::ARCH)
        })
        .ok_or("unsupported native authentication ABI")?;
    for (relative, expected) in family["files"].as_object().ok_or(FAILURE)? {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(directory.join("rootfs").join(relative))
            .map_err(|_| "unsupported native authentication ABI")?;
        let metadata = file.metadata().map_err(|_| FAILURE)?;
        if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
            return Err(FAILURE.into());
        }
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FAILURE)?;
        if Some(super::digest_hex(&bytes).as_str()) != expected.as_str() {
            return Err("unsupported native authentication ABI".into());
        }
    }
    Ok(())
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn number(&mut self) -> Result<u32, String> {
        let bytes = self.0.get(..4).ok_or(FAILURE)?;
        let value = u32::from_le_bytes(bytes.try_into().map_err(|_| FAILURE)?);
        self.0 = &self.0[4..];
        Ok(value)
    }
    fn text(&mut self) -> Result<String, String> {
        let length = self.number()? as usize;
        if length > 8192 {
            return Err(FAILURE.into());
        }
        let text = std::str::from_utf8(self.0.get(..length).ok_or(FAILURE)?)
            .map_err(|_| FAILURE)?
            .to_string();
        self.0 = &self.0[length..];
        Ok(text)
    }
}
#[derive(Default)]
struct Conversation {
    ready: bool,
    last: u32,
    pending: Option<Prompt>,
    authenticated: bool,
}
impl Conversation {
    fn event(&mut self, data: &[u8]) -> Result<Option<Event>, String> {
        let mut reader = Reader(data);
        let kind = reader.number()?;
        let result = if kind == 1 && !self.ready {
            self.ready = true;
            None
        } else {
            if !self.ready || self.authenticated {
                return Err(FAILURE.into());
            }
            let id = reader.number()?;
            if id <= self.last || self.pending.is_some() {
                return Err(FAILURE.into());
            }
            self.last = id;
            match kind {
                2 | 3 => {
                    let prompt_kind = if kind == 2 {
                        PromptKind::Dialog
                    } else {
                        match reader.number()? {
                            0 => PromptKind::Credentials,
                            1 => PromptKind::Code,
                            2 => PromptKind::CredentialsAndCode,
                            _ => return Err(FAILURE.into()),
                        }
                    };
                    let title = reader.text()?;
                    let message = reader.text()?;
                    let buttons = if kind == 2 {
                        let count = reader.number()?;
                        if count == 0 || count > 16 {
                            return Err(FAILURE.into());
                        }
                        (0..count)
                            .map(|id| {
                                Ok(Button {
                                    id,
                                    label: reader.text()?,
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()?
                    } else {
                        vec![
                            Button {
                                id: 0,
                                label: reader.text()?,
                            },
                            Button {
                                id: 2,
                                label: reader.text()?,
                            },
                        ]
                    };
                    let prompt = Prompt {
                        id,
                        kind: prompt_kind,
                        title,
                        message,
                        buttons,
                    };
                    self.pending = Some(prompt.clone());
                    Some(Event::Prompt { prompt })
                }
                4 if reader.number()? == 6 => {
                    self.authenticated = true;
                    Some(Event::Checking)
                }
                _ => return Err(FAILURE.into()),
            }
        };
        if !reader.0.is_empty() {
            return Err(FAILURE.into());
        }
        Ok(result)
    }
    fn reply(&mut self, input: Input) -> Result<Vec<u8>, String> {
        let prompt = self.pending.as_ref().ok_or(FAILURE)?;
        let mut bytes = Vec::new();
        let number = |v: u32, bytes: &mut Vec<u8>| bytes.extend_from_slice(&v.to_le_bytes());
        let text = |v: &str, bytes: &mut Vec<u8>| -> Result<(), String> {
            if v.is_empty() || v.as_bytes().contains(&0) {
                return Err(FAILURE.into());
            }
            number(v.len() as u32, bytes);
            bytes.extend_from_slice(v.as_bytes());
            Ok(())
        };
        match input {
            Input::Select { id, button }
                if id == prompt.id
                    && prompt.buttons.iter().any(|b| b.id == button)
                    && (prompt.kind == PromptKind::Dialog || button == 0) =>
            {
                number(10, &mut bytes);
                number(id, &mut bytes);
                number(button, &mut bytes);
            }
            Input::Credentials {
                id,
                username,
                password,
            } if id == prompt.id
                && prompt.kind == PromptKind::Credentials
                && username.len() < 1024
                && password.len() < 4096 =>
            {
                number(11, &mut bytes);
                number(id, &mut bytes);
                text(&username, &mut bytes)?;
                text(&password, &mut bytes)?;
            }
            Input::CredentialsAndCode {
                id,
                username,
                password,
                code,
            } if id == prompt.id
                && prompt.kind == PromptKind::CredentialsAndCode
                && username.len() < 1024
                && password.len() < 4096
                && code.len() == 6
                && code.bytes().all(|c| c.is_ascii_digit()) =>
            {
                number(13, &mut bytes);
                number(id, &mut bytes);
                text(&username, &mut bytes)?;
                text(&password, &mut bytes)?;
                text(&code, &mut bytes)?;
            }
            Input::Code { id, code }
                if id == prompt.id
                    && prompt.kind == PromptKind::Code
                    && code.len() == 6
                    && code.bytes().all(|c| c.is_ascii_digit()) =>
            {
                number(12, &mut bytes);
                number(id, &mut bytes);
                text(&code, &mut bytes)?;
            }
            _ => return Err(FAILURE.into()),
        }
        self.pending = None;
        let mut framed = (bytes.len() as u32).to_le_bytes().to_vec();
        framed.extend_from_slice(&bytes);
        melba_auth::erase(&mut bytes);
        Ok(framed)
    }
}
fn inherit(fd: RawFd) -> Result<(), std::io::Error> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn run(
    emit: impl FnMut(Event) -> Result<(), String>,
    publish: impl Fn(&str) -> Result<(), String>,
    reuse: bool,
    stop_daemon: bool,
) -> Result<(), String> {
    run_attempt(
        emit,
        publish,
        crate::musickit::validate_token,
        verify_payload,
        Duration::from_secs(600),
        Options { reuse, stop_daemon },
    )
}

#[derive(Default)]
struct Options {
    reuse: bool,
    stop_daemon: bool,
}

fn run_attempt(
    mut emit: impl FnMut(Event) -> Result<(), String>,
    publish: impl Fn(&str) -> Result<(), String>,
    validate: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    verify: impl Fn(&Path) -> Result<(), String>,
    timeout: Duration,
    options: Options,
) -> Result<(), String> {
    let validate = std::sync::Arc::new(validate);
    let binary = crate::wrapper::installed().ok_or(FAILURE)?;
    let directory = binary.parent().ok_or(FAILURE)?;
    // A GUI account change may stop this daemon's managed child, never a
    // process discovered by a listening port. Old/foreign daemons simply do
    // not implement this private extension; the lock/port checks still fail closed.
    if let Some(conn) = options
        .stop_daemon
        .then(dbus::blocking::Connection::new_session)
        .and_then(Result::ok)
    {
        let proxy = conn.with_proxy(
            "org.mpris.MediaPlayer2.melba",
            "/org/mpris/MediaPlayer2",
            Duration::from_secs(2),
        );
        let _: Result<(), _> = proxy.method_call("org.mpris.MediaPlayer2.Player", "Stop", ());
        let _: Result<(), _> = proxy.method_call(
            "org.mpris.MediaPlayer2.Player",
            "MelbaStopManagedWrapper",
            (),
        );
    }
    let lease = crate::wrapper::session_lock()?;
    verify(directory)?;
    let bridge = prepare_bridge(directory)?;
    let paths = token_paths(directory);
    let before: Vec<_> = paths.iter().map(|p| identity(p)).collect();
    let (mut control, peer) = UnixStream::pair().map_err(|_| FAILURE)?;
    control.set_nonblocking(true).map_err(|_| FAILURE)?;
    let mut command = crate::wrapper::process::command(directory, &binary, false)?;
    command
        .current_dir(directory)
        .args(["-L", ":"])
        .env("MELBA_AUTH_FD", peer.as_raw_fd().to_string())
        .env("MELBA_AUTH_REUSE", if options.reuse { "1" } else { "0" })
        .env("LD_PRELOAD", bridge.file_name().ok_or(FAILURE)?)
        .env("LD_LIBRARY_PATH", bridge.parent().ok_or(FAILURE)?);
    let fd = peer.as_raw_fd();
    let lock_fd = lease.as_raw_fd();
    unsafe {
        command.pre_exec(move || {
            inherit(fd)?;
            inherit(lock_fd)?;
            libc::umask(0o077);
            Ok(())
        });
    }
    // An explicit sign-in invalidates the previous binding up front: the user
    // is replacing the account, so the old token must not silently stand in.
    // A *restore* only re-uses an existing binding, so it must not write a
    // tombstone before it starts: a restore that fails early (wrapper busy,
    // no session yet, transient error) used to destroy a valid binding and
    // force a fresh interactive sign-in — and with it the managed wrapper.
    if !options.reuse {
        super::begin_binding()?;
    }
    let child = spawn(&mut command)?;
    drop(peer);
    let mut conversation = Conversation::default();
    let mut native = Vec::new();
    let mut line = Vec::new();
    let deadline = Instant::now() + timeout;
    let ready_deadline = Instant::now() + Duration::from_secs(30);
    let mut validation = None;
    let mut validated = false;
    let mut prepared = None;
    loop {
        if Instant::now() > deadline
            || (!conversation.ready && Instant::now() > ready_deadline)
            || child.status()?.is_some()
        {
            return Err(FAILURE.into());
        }
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: control.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, 50) } < 0 {
            return Err(FAILURE.into());
        }
        // Cancellation is considered before a result from this iteration.
        if fds[0].revents != 0 {
            let mut chunk = [0u8; 4096];
            let n = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
            if n <= 0 {
                return Err(FAILURE.into());
            }
            for byte in &chunk[..n as usize] {
                if *byte == b'\n' {
                    let input = serde_json::from_slice::<Input>(&line).map_err(|_| FAILURE);
                    melba_auth::erase(&mut line);
                    line.clear();
                    let input = input?;
                    if matches!(input, Input::Cancel) {
                        return Err(FAILURE.into());
                    }
                    if matches!(input, Input::Commit) {
                        let token: melba_auth::Secret = prepared.take().ok_or(FAILURE)?;
                        drop(child);
                        publish(&token)?;
                        drop(lease);
                        return emit(Event::Complete);
                    }
                    let mut reply = conversation.reply(input)?;
                    control.set_nonblocking(false).map_err(|_| FAILURE)?;
                    control
                        .set_write_timeout(Some(Duration::from_secs(1)))
                        .map_err(|_| FAILURE)?;
                    let result = control.write_all(&reply);
                    melba_auth::erase(&mut reply);
                    control.set_nonblocking(true).map_err(|_| FAILURE)?;
                    result.map_err(|_| FAILURE)?;
                } else {
                    if line.len() >= MAX_PACKET {
                        return Err(FAILURE.into());
                    }
                    line.push(*byte);
                }
            }
            melba_auth::erase(&mut chunk);
        }
        if fds[1].revents != 0 {
            let mut chunk = [0u8; 4096];
            let n = control.read(&mut chunk).map_err(|_| FAILURE)?;
            if n == 0 || native.len() + n > MAX_PACKET + 4 {
                return Err(FAILURE.into());
            }
            native.extend_from_slice(&chunk[..n]);
            while native.len() >= 4 {
                let length =
                    u32::from_le_bytes(native[..4].try_into().map_err(|_| FAILURE)?) as usize;
                if !(4..=MAX_PACKET).contains(&length) {
                    return Err(FAILURE.into());
                }
                if native.len() < length + 4 {
                    break;
                }
                if let Some(event) = conversation.event(&native[4..length + 4])? {
                    emit(event)?;
                }
                native.drain(..length + 4);
            }
        }
        if conversation.authenticated && !validated && validation.is_none() {
            for (path, previous) in paths.iter().zip(&before) {
                // A restore reuses the session the wrapper already has, so its
                // token file is often unchanged; the token is still validated
                // before publication. An explicit sign-in must produce a fresh
                // token, so only a changed file counts there.
                let current = identity(path);
                if current.is_some() && (current != *previous || options.reuse) {
                    if let Some(token) = read_token(path) {
                        let (tx, rx) = std::sync::mpsc::sync_channel(1);
                        let validate = std::sync::Arc::clone(&validate);
                        std::thread::spawn(move || {
                            let result = validate(&token).map(|_| token);
                            let _ = tx.send(result);
                        });
                        validation = Some(rx);
                        break;
                    }
                }
            }
        }
        if let Some(receiver) = &validation {
            match receiver.try_recv() {
                Ok(Ok(token)) => {
                    validated = true;
                    prepared = Some(melba_auth::Secret::from(token));
                    validation = None;
                    emit(Event::ReadyToCommit)?;
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(FAILURE.into())
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn number(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        number(bytes, value.len() as u32);
        bytes.extend_from_slice(value.as_bytes());
    }
    fn offered() -> Conversation {
        let mut c = Conversation::default();
        c.event(&1u32.to_le_bytes()).unwrap();
        let mut packet = Vec::new();
        number(&mut packet, 2);
        number(&mut packet, 1);
        text(&mut packet, "Verification");
        text(&mut packet, "Choose an offered method");
        number(&mut packet, 3);
        for label in ["Cancel", "Text ••12", "Call ••12"] {
            text(&mut packet, label);
        }
        assert!(matches!(
            c.event(&packet).unwrap(),
            Some(Event::Prompt { .. })
        ));
        c
    }
    #[test]
    fn isolated_native_peer_round_trip() {
        if std::env::var("MELBA_NATIVE_TEST_CHILD").as_deref() == Ok("1") {
            // This branch exists only in the test binary. The SDK peer and
            // server validation edge are controlled; production has no bypass.
            let scenario = std::env::var("FAKE_SCENARIO").unwrap();
            let reject = scenario == "validation-rejected";
            let result = run_attempt(
                super::super::emit,
                super::super::publish,
                move |_| {
                    std::fs::write(
                        std::env::var("FAKE_VALIDATION_TRACE").unwrap(),
                        "validator-called",
                    )
                    .unwrap();
                    if reject {
                        Err("fixture server rejected token".into())
                    } else {
                        Ok(())
                    }
                },
                |_| Ok(()),
                Duration::from_secs(4),
                Options {
                    reuse: matches!(scenario.as_str(), "resume" | "restore-failed"),
                    stop_daemon: false,
                },
            );
            if result.is_err() {
                let _ = super::super::emit(Event::Failed {
                    message: "fixture attempt rejected".into(),
                });
            }
            if scenario == "restore-failed" {
                // This restore failed before it reached the native session and
                // no binding was enrolled, so it must not have written one:
                // that tombstone used to destroy a valid binding and force a
                // fresh sign-in (and block the managed wrapper) on every
                // startup whose restore failed.
                assert!(
                    super::super::check_managed_binding().is_ok(),
                    "a restore that never authenticated invalidated the binding"
                );
            } else {
                assert_eq!(
                    super::super::check_managed_binding().is_ok(),
                    result.is_ok()
                );
            }
            assert_eq!(
                result.is_ok(),
                matches!(
                    scenario.as_str(),
                    "sms" | "voice" | "wrong-code" | "cached-code" | "device" | "resume"
                )
            );
        } else {
            let status = std::process::Command::new("python3")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/native_auth_peer.py"
                ))
                .arg(std::env::current_exe().unwrap())
                .status()
                .unwrap();
            assert!(status.success(), "isolated native peer contract failed");
        }
    }

    /// The real payload's loader cannot resolve libc symbols for a preload --
    /// measured: it reports `cannot locate symbol "write"` and the bridge never
    /// loads. A future edit that reintroduces a libc call would only show up in
    /// a live payload run, so the built artifact is checked directly here.
    #[test]
    fn the_staged_bridge_imports_no_symbols_and_needs_no_library() {
        let artifact = concat!(env!("OUT_DIR"), "/melba-auth.so");
        let run = |tool: &str, arguments: &[&str]| {
            std::process::Command::new(tool)
                .args(arguments)
                .arg(artifact)
                .output()
        };
        let Ok(dynamic) = run("readelf", &["-d"]) else {
            eprintln!("readelf is unavailable: the bridge ELF gate is skipped");
            return;
        };
        assert!(dynamic.status.success(), "readelf failed on the bridge");
        let dynamic = String::from_utf8_lossy(&dynamic.stdout);
        assert!(
            !dynamic.contains("NEEDED"),
            "the bridge must not depend on another library:\n{dynamic}"
        );
        let Ok(undefined) = run("nm", &["-D", "--undefined-only"]) else {
            eprintln!("nm is unavailable: the bridge import gate is skipped");
            return;
        };
        let undefined = String::from_utf8_lossy(&undefined.stdout);
        let imported: Vec<&str> = undefined
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert!(
            imported.is_empty(),
            "the bridge must not import symbols (the payload loader cannot resolve them): {imported:?}"
        );
    }

    #[test]
    fn offered_sms_and_voice_indices_are_preserved() {
        for button in [1, 2] {
            let mut c = offered();
            let reply = c.reply(Input::Select { id: 1, button }).unwrap();
            assert_eq!(&reply[12..16], &button.to_le_bytes());
        }
    }
    #[test]
    fn stale_and_unoffered_choices_are_rejected() {
        assert!(offered().reply(Input::Select { id: 0, button: 1 }).is_err());
        assert!(offered().reply(Input::Select { id: 1, button: 3 }).is_err());
    }
    #[test]
    fn native_success_requires_a_completed_owned_conversation() {
        let mut bytes = Vec::new();
        number(&mut bytes, 4);
        number(&mut bytes, 2);
        number(&mut bytes, 6);
        assert!(Conversation::default().event(&bytes).is_err());
        assert!(offered().event(&bytes).is_err());
        let mut c = offered();
        c.reply(Input::Select { id: 1, button: 1 }).unwrap();
        assert!(matches!(c.event(&bytes).unwrap(), Some(Event::Checking)));
        assert!(c.authenticated);
    }
}
