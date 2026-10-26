//! A GUI attempt owns its native child and accepts only that child's result.
#[cfg(target_arch = "x86_64")]
mod native;
use melba_auth::{Event, Proof, PROOF_FILE};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn proof_path() -> Option<PathBuf> {
    crate::auth::token_path().map(|p| p.with_file_name(PROOF_FILE))
}
const TOMBSTONE: &str = r#"{"version":1,"token_sha256":""}"#;

// Serialize binding writes, not the whole SDK attempt. In particular, a failed
// spawn must not restore an old proof over a concurrent explicit `auth clear`.
fn binding_lock() -> Result<Option<std::fs::File>, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = proof_path() else {
        return Ok(None);
    };
    crate::auth::private_dir(path.parent().ok_or("no private configuration directory")?)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))
        .map_err(|_| "cannot lock account binding")?;
    if !file
        .metadata()
        .map_err(|_| "cannot inspect account binding lock")?
        .is_file()
    {
        return Err("account binding lock is not a regular file".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("account binding is busy; retry after the current sign-in transition".into());
    }
    Ok(Some(file)) // Rust opens it close-on-exec; the SDK must not inherit it.
}
fn read_binding() -> Result<Option<String>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(path) = proof_path() else {
        return Ok(None);
    };
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("cannot read account binding".into()),
    };
    if !file
        .metadata()
        .map_err(|_| "cannot inspect account binding")?
        .is_file()
    {
        return Err("account binding is not a regular file".into());
    }
    let mut text = String::new();
    file.take(513)
        .read_to_string(&mut text)
        .map_err(|_| "cannot read account binding")?;
    if text.len() > 512 {
        return Err("account binding is too large".into());
    }
    Ok(Some(text))
}
fn store_binding(proof: &str) -> Result<(), String> {
    crate::auth::store(
        &proof_path().ok_or("no private configuration directory")?,
        proof,
    )
}
pub(crate) fn invalidate_binding() -> Result<(), String> {
    let _lock = binding_lock()?;
    if proof_path().is_some_and(|p| p.symlink_metadata().is_ok()) {
        store_binding(TOMBSTONE)?;
    }
    Ok(())
}
fn begin_binding() -> Result<(), String> {
    let _lock = binding_lock()?;
    store_binding(TOMBSTONE)
}
/// A failed exec did not begin provisioning. Keep the old binding in that case,
/// but tombstone it BEFORE a successfully launched child can mutate the SDK.
/// Native GUI sign-in deliberately retains its distinct fresh-attempt policy.
pub(crate) fn spawn_provisioning(
    cmd: &mut std::process::Command,
) -> Result<std::process::Child, String> {
    let _lock = binding_lock()?;
    let previous = read_binding()?;
    if previous.is_some() {
        store_binding(TOMBSTONE)?;
    }
    match cmd.spawn() {
        Ok(child) => Ok(child),
        Err(error) => {
            if let Some(proof) = previous {
                store_binding(&proof)
                    .map_err(|_| "spawn failed and the saved binding could not be restored")?;
            }
            Err(format!("spawn wrapper: {error}"))
        }
    }
}
/// `auth clear` means "no local sign-in state". A tombstone from a failed
/// attempt must not keep the managed wrapper out of use after the user
/// explicitly cleared the token, so the binding is forgotten with it.
pub fn forget_binding() -> Result<(), String> {
    let Some(path) = proof_path() else {
        return Ok(());
    };
    let _lock = binding_lock()?;
    // remove_file unlinks the final component itself: no symlink is followed.
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
pub(crate) fn check_managed_binding() -> Result<(), String> {
    if proof_path().is_some_and(|p| p.symlink_metadata().is_ok()) && !has_native_binding() {
        return Err("complete GUI sign-in before using the managed wrapper".into());
    }
    Ok(())
}
fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn token_digest(token: &str) -> String {
    digest_hex(token.as_bytes())
}
fn native_required() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        crate::wrapper::managed()
            || (crate::config::wrapper_url().is_none() && crate::wrapper::installed().is_some())
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// A cached binding only. It must never stand in for successful NEW login.
fn has_native_binding() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        let Some(token) = crate::auth::get() else {
            return false;
        };
        read_binding()
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<Proof>(&text).ok())
            .is_some_and(|p| {
                p.version == 1
                    && p.token_sha256 == token_digest(&token)
                    && native::matches_current_account(&token)
            })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}
fn emit(event: Event) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&event).map_err(|_| "cannot encode sign-in event")?;
    bytes.push(b'\n');
    if bytes.len() > melba_auth::MAX_PACKET {
        return Err("sign-in event too large".into());
    }
    // Do not block forever writing a dialog after the GUI has cancelled.
    let flags = unsafe { libc::fcntl(1, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(1, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("sign-in channel closed".into());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut position = 0;
    while position < bytes.len() {
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: 0,
                revents: 0,
            },
            libc::pollfd {
                fd: 1,
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        if std::time::Instant::now() > deadline
            || unsafe { libc::poll(fds.as_mut_ptr(), 2, 50) } < 0
            || fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0
        {
            return Err("sign-in channel closed".into());
        }
        if fds[1].revents & libc::POLLOUT != 0 {
            let n = unsafe {
                libc::write(1, bytes[position..].as_ptr().cast(), bytes.len() - position)
            };
            if n <= 0 {
                return Err("sign-in channel closed".into());
            }
            position += n as usize;
        }
    }
    Ok(())
}
fn publish(token: &str) -> Result<(), String> {
    // Publish the binding first. If replacing the token fails, the previous
    // token remains untouched and cannot match this new-account binding.
    let proof = serde_json::to_string(&Proof {
        version: 1,
        token_sha256: token_digest(token),
    })
    .map_err(|_| "cannot encode account binding")?;
    let _lock = binding_lock()?;
    store_binding(&proof)?;
    crate::auth::set(token)
}

pub fn run(restore: bool) -> Result<(), String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() || std::io::stdout().is_terminal() {
        return Err("GUI sign-in requires private pipes".into());
    }
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
        return Err("cannot protect transient sign-in input".into());
    }
    let result = if native_required() {
        // An explicit sign-in is always a fresh SDK attempt. A startup restore
        // keeps the session the user already has, as long as Apple still
        // accepts the saved token: the managed wrapper is normally running
        // (the daemon starts it for playback and it holds the session lock), so
        // a native restore attempt cannot take that lock and used to leave the
        // whole interface signed out with no library, artwork or playback.
        // Only a missing or rejected token falls through to the native flow.
        #[cfg(target_arch = "x86_64")]
        {
            if !restore {
                native::run(emit, publish, false, true)
            } else {
                match crate::auth::get()
                    .filter(|token| crate::musickit::validate_token(token).is_ok())
                {
                    Some(token) => {
                        publish(&token)?;
                        emit(Event::Complete)
                    }
                    None => native::run(emit, publish, true, false),
                }
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = restore;
            emit(Event::Browser {
                url: "https://music.apple.com".into(),
            })?;
            let _ = std::process::Command::new("xdg-open")
                .arg("https://music.apple.com")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            crate::auth::watch_browser(std::time::Duration::from_secs(600), |token| {
                crate::musickit::validate_token(token)?;
                emit(Event::ReadyToCommit)
            })
            .and_then(|_| emit(Event::Complete))
        }
    } else if restore {
        let valid =
            crate::auth::get().is_some_and(|token| crate::musickit::validate_token(&token).is_ok());
        emit(if valid {
            Event::Complete
        } else {
            Event::SignedOut
        })
    } else {
        // The existing exact-origin, validated browser path remains the light
        // route for installations without a local managed wrapper.
        emit(Event::Browser {
            url: "https://music.apple.com".into(),
        })?;
        let _ = std::process::Command::new("xdg-open")
            .arg("https://music.apple.com")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        crate::auth::watch_browser(std::time::Duration::from_secs(600), |token| {
            crate::musickit::validate_token(token)?;
            emit(Event::ReadyToCommit)?;
            use std::io::{BufRead, Read};
            let mut line = Vec::new();
            std::io::stdin()
                .lock()
                .take(melba_auth::MAX_PACKET as u64 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|_| "sign-in channel closed")?;
            let input = serde_json::from_slice::<melba_auth::Input>(&line);
            melba_auth::erase(&mut line);
            if !matches!(input, Ok(melba_auth::Input::Commit)) {
                return Err("sign-in cancelled".into());
            }
            Ok(())
        })
        .and_then(|_| emit(Event::Complete))
    };
    if result.is_err() {
        // Never put SDK responses, credentials, tokens, or raw subprocess
        // output into the protocol error or logs.
        let message = if result
            .as_ref()
            .err()
            .is_some_and(|e| e == "unsupported native authentication ABI")
        {
            "This wrapper build is not supported by GUI sign-in. Use the pinned x86_64 payload from wrapper setup. Custom SDK versions and other architectures need compatibility verification first."
        } else {
            "Sign-in did not complete. Your previous token was kept. Check that the local wrapper is installed and not in use, then try again."
        };
        let _ = emit(Event::Failed {
            message: message.into(),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_spawn_serializes_binding_writers() {
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        use std::os::unix::process::CommandExt;
        const CASE: &str = "auth_gui::tests::failed_spawn_serializes_binding_writers";
        if std::env::var_os("MELBA_TEST_BINDING_ISOLATED").is_none() {
            let root = std::env::temp_dir().join(format!("melba-binding-{}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CASE, "--nocapture"])
                .env("MELBA_TEST_BINDING_ISOLATED", "1")
                .env("HOME", &root)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_DATA_HOME", root.join("data"))
                .env("XDG_CACHE_HOME", root.join("cache"))
                .output()
                .unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        store_binding("fixture-binding").unwrap();
        let (mut control, gate) = UnixStream::pair().unwrap();
        control
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let worker = std::thread::spawn(move || {
            let fd = gate.as_raw_fd();
            let mut cmd = std::process::Command::new("/no-such-melba-test-executable");
            unsafe {
                cmd.pre_exec(move || {
                    let mut byte = 1u8;
                    if libc::write(fd, (&byte as *const u8).cast(), 1) != 1
                        || libc::read(fd, (&mut byte as *mut u8).cast(), 1) != 1
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let result = spawn_provisioning(&mut cmd);
            drop(gate);
            assert!(result.is_err());
        });
        control.read_exact(&mut [0]).unwrap();
        let pending = read_binding().unwrap();
        let clear_during_spawn = forget_binding();
        control.write_all(&[1]).unwrap();
        worker.join().unwrap();
        assert_eq!(pending.as_deref(), Some(TOMBSTONE));
        assert!(
            clear_during_spawn.is_err(),
            "a concurrent binding writer bypassed the spawn transaction"
        );
        assert_eq!(read_binding().unwrap().as_deref(), Some("fixture-binding"));
        forget_binding().unwrap();
        assert!(read_binding().unwrap().is_none());
    }

    #[test]
    fn native_binding_is_a_standard_digest_not_a_token() {
        assert_eq!(
            token_digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
