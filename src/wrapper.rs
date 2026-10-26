//! Managed ALAC decrypt service lifecycle.
//!
//! `wrapper_mode = "managed"` in config.json makes melbad run a local
//! decrypt service itself: binaries live under
//! `~/.local/share/melba/wrapper/`, spawned on demand the first time an
//! ALAC track is opened and reused afterwards. The optional installer
//! separately acquires a runtime containing Apple and Android libraries;
//! a local APK override is optional. That staging does not establish
//! permission to use or redistribute those third-party libraries.
//!
//! The managed service speaks the key-extraction protocol on its HTTP
//! key port: `GET ?adamId=<id>&uri=<skd>` -> {"contentKey": …}, which is
//! the same shape `wrapper_url = http://…` already accepts.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
pub(crate) mod process;
use process::ProcessGroup;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

static CHILD: OnceLock<Mutex<Option<ProcessGroup>>> = OnceLock::new();

/// Serialize account mutations with managed playback starts. The service
/// inherits this open-file-description lock until its last process exits.
pub(crate) fn session_lock() -> Result<std::fs::File, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let directory = dir().ok_or("no private wrapper directory")?;
    crate::auth::private_dir(&directory)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(".session.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("the managed wrapper is in use; stop playback before signing in".into());
    }
    Ok(file)
}

fn dir() -> Option<PathBuf> {
    crate::config::xdg_dir("XDG_DATA_HOME", ".local/share").map(|p| p.join("melba/wrapper"))
}

/// Installed service binary path, if setup has been run.
pub fn installed() -> Option<PathBuf> {
    binary()
}

/// The managed service entrypoint: `run` is the launcher written by
/// install-wrapper.sh (native upstream bundle per host arch);
/// `wrapper*` fallbacks cover manual layouts.
fn binary() -> Option<PathBuf> {
    let d = dir()?;
    for name in ["run", "wrapper-lite-qemu", "wrapper-lite", "wrapper"] {
        let p = d.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Key-service URL once the managed service is up.
pub fn key_url() -> String {
    // upstream `main` serves the key-extraction endpoint on its dedicated
    // key port — `-K/--key-port`, default 40020: the binary's own help
    // and its `[!] listening key request on %s:%d` / `[.] key request:
    // adamId=%s uri=%s` / `[.] key response: … contentKey` strings all
    // hang off that port (10020 is a separate decrypt listener).
    "http://127.0.0.1:40020".into()
}

/// Sample-decrypt endpoint handed to `open_alac` — the wrapper's `-D`
/// port (default 10020): a persistent TCP service that takes
/// `[1B len][adam][1B len][uri]` then streams `[u32 LE size][cipher]`
/// frames answered with equal-length plaintext. The key port's
/// `contentKey` is a CKC blob clients cannot turn into a raw key
/// without Apple's derivation, so decryption goes through here.
pub fn decrypt_url() -> String {
    "tcp://127.0.0.1:10020".into()
}

/// True when config asks melbad to run the decrypt service itself.
pub fn managed() -> bool {
    crate::config::config()["wrapper_mode"].as_str() == Some("managed")
}

/// TCP reachability probe for a configured wrapper — accepts the same
/// tcp://host:port | host:port | http(s)://host[:port] forms as open_alac.
pub fn check_wrapper(url: &str) -> Result<String, String> {
    use std::net::ToSocketAddrs;
    let host = url
        .trim_start_matches("tcp://")
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let host = host.split('/').next().unwrap_or(host).to_string();
    let t = std::time::Duration::from_secs(3);
    host.to_socket_addrs()
        .ok()
        .and_then(|mut it| it.find_map(|a| std::net::TcpStream::connect_timeout(&a, t).ok()))
        .map(|_| format!("reachable at {host}"))
        .ok_or_else(|| format!("{host}: connection failed"))
}

/// Human-readable state for `melbad wrapper status`.
pub fn status() -> String {
    let mut out = match dir() {
        Some(d) => format!("dir: {}", d.display()),
        None => "dir: NONE (no XDG_DATA_HOME or HOME)".to_string(),
    };
    match binary() {
        Some(b) => out += &format!("\nbinary: {}", b.display()),
        None => out += "\nbinary: MISSING — run `melbad wrapper setup`",
    }
    let managed = alive() || !managed_pids().is_empty();
    out += &format!("\nrunning: {}", if managed { "yes" } else { "no" });
    match check_wrapper(&key_url()) {
        Ok(s) => out += &format!("\nkey port: {s}"),
        Err(e) => out += &format!("\nkey port: {e}"),
    }
    out
}

fn alive() -> bool {
    cell()
        .lock()
        .ok()
        .map(|mut c| running(&mut c))
        .unwrap_or(false)
}

/// Managed processes that survive a separate `melbad wrapper start` parent.
/// Match only executables under the managed directory; a Gradle/other
/// wrapper or a user's external service must never be signalled.
fn managed_pids() -> Vec<u32> {
    let Some(d) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        if exe.starts_with(&d) {
            pids.push(pid);
        }
    }
    pids
}

fn stop_managed_processes() -> Result<(), String> {
    let pids = managed_pids();
    for &pid in &pids {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !managed_pids().is_empty() {
        std::thread::sleep(Duration::from_millis(100));
    }
    for &pid in &managed_pids() {
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    Ok(())
}

fn cell() -> &'static Mutex<Option<ProcessGroup>> {
    CHILD.get_or_init(|| Mutex::new(None))
}

/// Latched when a managed service we spawned died before its key port
/// answered. Once set, `try_ready` stops re-probing — a wrapper that
/// exits early (e.g. FairPlay rejects the account) would otherwise cost
/// its whole startup window on every track open. Cleared by the next
/// successful `spawn_locked`, i.e. an explicit `melbad wrapper start`.
static EARLY_EXIT: AtomicBool = AtomicBool::new(false);

/// True while `c` names a still-running child (also reaps zombies).
fn running(c: &mut Option<ProcessGroup>) -> bool {
    matches!(c.as_ref().map(|x| x.status()), Some(Ok(None)))
}

/// Spawn the service and store the child under `guard` — callers hold the
/// CHILD mutex across the alive check + spawn so two racing opens can't
/// each start a copy fighting over the same ports.
fn spawn_locked(guard: &mut Option<ProcessGroup>, detached: bool) -> Result<String, String> {
    let bin = binary().ok_or_else(|| {
        "ALAC needs the decrypt service — run `melbad wrapper setup` first \
         (or set wrapper_url for an external one)"
            .to_string()
    })?;
    let d = dir().ok_or("no XDG_DATA_HOME or HOME")?;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let lease = session_lock()?;
    let fd = lease.as_raw_fd();
    let log = d.join("wrapper.log");
    let lf = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    // `run` already supplies -H 127.0.0.1 — passing it again makes the
    // wrapper binary reject the duplicate and exit early
    let mut command = process::command(&d, &bin, detached)?;
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = ProcessGroup::spawn(
        command
            .current_dir(&d)
            .stdout(Stdio::from(lf.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(lf)),
    )
    .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    *guard = Some(child);
    EARLY_EXIT.store(false, Ordering::SeqCst);
    Ok(log.display().to_string())
}

/// Bounded-wait variant for the playback path: probe the port, kick off
/// the service when it is not already starting, then give it a short
/// window to bind. A provisioned service opens its ports in ~10-20s, so
/// the first open after daemon start still gets ALAC; an unprovisioned
/// first boot can take minutes and falls back to AAC after the window.
/// `ensure` is the blocking CLI form of the same operation.
pub fn try_ready() -> Result<String, String> {
    crate::auth_gui::check_managed_binding()?;
    if check_wrapper(&key_url()).is_ok() {
        return Ok(decrypt_url());
    }
    if EARLY_EXIT.load(Ordering::SeqCst) {
        return Err(
            "managed wrapper exited early — see wrapper.log (retry: `melbad wrapper start`)".into(),
        );
    }
    {
        // hold the lock across check+spawn — two racing opens must not
        // each launch a service competing for the same ports
        let mut guard = cell().lock().map_err(|e| e.to_string())?;
        if !running(&mut guard) && managed_pids().is_empty() {
            spawn_locked(&mut guard, false)?;
        }
    }
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
        if check_wrapper(&key_url()).is_ok() {
            return Ok(decrypt_url());
        }
        if !alive() && managed_pids().is_empty() {
            EARLY_EXIT.store(true, Ordering::SeqCst);
            return Err("managed wrapper exited early — see wrapper.log".into());
        }
    }
    Err("managed wrapper starting".into())
}

/// Ensure the managed service is listening; returns its key-service
/// URL. Installs nothing — missing bits tell the user to run setup.
/// Blocks up to 240s: for `melbad wrapper start`, never the playback path
/// (use `try_ready` there).
pub fn ensure() -> Result<String, String> {
    crate::auth_gui::check_managed_binding()?;
    if check_wrapper(&key_url()).is_ok() {
        return Ok(decrypt_url());
    }
    let log = {
        let mut guard = cell().lock().map_err(|e| e.to_string())?;
        if running(&mut guard) || !managed_pids().is_empty() {
            // spawned but not listening yet — caller sees the error and the
            // next open retries.
            return Err("managed wrapper starting".into());
        }
        spawn_locked(&mut guard, true)?
    };

    // The key port binds only after provisioning/login completes —
    // wait generously (first runs configure FairPlay and write the
    // account DBs). Early-returns the moment the port answers.
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(240) {
        if check_wrapper(&key_url()).is_ok() {
            return Ok(decrypt_url());
        }
        std::thread::sleep(Duration::from_secs(1));
        if !alive() && managed_pids().is_empty() {
            EARLY_EXIT.store(true, Ordering::SeqCst);
            return Err(format!("managed wrapper exited early — see {log}"));
        }
    }
    Err(format!(
        "managed wrapper did not open {} in 240s — see {log}",
        key_url()
    ))
}

/// Stop the managed service (leaves files in place).
pub fn stop() -> Result<(), String> {
    {
        let mut c = cell().lock().map_err(|e| e.to_string())?;
        drop(c.take());
    }
    stop_managed_processes()?;
    EARLY_EXIT.store(false, Ordering::SeqCst);
    if check_wrapper(&key_url()).is_ok() {
        return Err(format!(
            "a service is still reachable at {}; it is not a managed child, so it was not stopped",
            key_url()
        ));
    }
    Ok(())
}

/// Read without terminal echo. rpassword restores terminal attributes when
/// returning; defer its Ctrl-C signal until that restoration has completed.
pub fn read_password(prompt: &str) -> std::io::Result<String> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        std::io::stderr().write_all(prompt.as_bytes())?;
        std::io::stderr().flush()?;
        // Preserve stdin's shared buffer and consume the entire LF/CRLF line;
        // the next reader must receive the 2FA code, never the password or the
        // remaining half of its delimiter. A redirected stream has no echo.
        let mut password = String::new();
        if std::io::stdin().read_line(&mut password)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "no password input",
            ));
        }
        if password.ends_with('\n') {
            password.pop();
        }
        if password.ends_with('\r') {
            password.pop();
        }
        return Ok(password);
    }

    use nix::sys::signal::{SigSet, SigmaskHow, Signal};
    struct RestoreMask(SigSet);
    impl Drop for RestoreMask {
        fn drop(&mut self) {
            let _ = self.0.thread_set_mask();
        }
    }
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGINT);
    let old = blocked
        .thread_swap_mask(SigmaskHow::SIG_BLOCK)
        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
    let restore = RestoreMask(old);
    let config = rpassword::ConfigBuilder::new()
        .input_file_path("/dev/stdin")
        .output_file_path("/dev/stderr")
        .build();
    let result = rpassword::prompt_password_with_config(prompt, config);
    drop(restore);
    result
}

/// Apple ID password for `login`. Prefers `MELBA_WRAPPER_PASSWORD` (then
/// removes it from this process) and otherwise reads one line from the
/// terminal with echo disabled. Never takes the password from argv.
pub fn read_login_password(user: &str) -> Result<String, String> {
    if std::env::var_os("MELBA_WRAPPER_PASSWORD").is_some() {
        let p = std::env::var("MELBA_WRAPPER_PASSWORD")
            .map_err(|_| "MELBA_WRAPPER_PASSWORD is not valid UTF-8".to_string());
        clear_password_env();
        let p = p?;
        if p.is_empty() {
            return Err("MELBA_WRAPPER_PASSWORD is set but empty".into());
        }
        return Ok(p);
    }
    read_password(&format!("Apple ID password for {user}: "))
        .map_err(|_| "failed to read password".to_string())
}

fn clear_password_env() {
    let key = std::ffi::CString::new("MELBA_WRAPPER_PASSWORD").unwrap();
    unsafe { libc::unsetenv(key.as_ptr()) };
}

const SCRUB_MARKER: &[u8] = b"melba-login-fd-v2";
/// Inherited across `run` → wrapper → bionic main. Not cloexec.
const MELBA_LOGIN_FD: i32 = 250;
/// Longer than this does not fit in the shim's buffer; refuse rather than
/// authenticate a prefix.
const MAX_LOGIN_CRED: usize = 4095;
/// How long a `-L` process may run without the shim confirming before it
/// is killed. Preload init is early; this only covers a stuck or missing
/// shim, not FairPlay startup.
const SCRUB_GRACE: Duration = Duration::from_secs(2);

fn argv_scrub_ready(lib64: &Path) -> bool {
    std::fs::read(lib64.join("argv-scrub.so"))
        .map(|buf| buf.windows(SCRUB_MARKER.len()).any(|w| w == SCRUB_MARKER))
        .unwrap_or(false)
}

fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn ppid_of(status: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(status);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("PPid:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// `root` and every process whose host PPid chain leads to it.
fn proc_tree(root: u32) -> Vec<(u32, u32)> {
    let mut all: Vec<(u32, u32)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for ent in rd.flatten() {
            let Ok(pid) = ent.file_name().to_str().unwrap_or("").parse::<u32>() else {
                continue;
            };
            let Ok(status) = std::fs::read(format!("/proc/{pid}/status")) else {
                continue;
            };
            if let Some(ppid) = ppid_of(&status) {
                all.push((pid, ppid));
            }
        }
    }
    let mut ids = vec![root];
    loop {
        let before = ids.len();
        for &(pid, ppid) in &all {
            if ids.contains(&ppid) && !ids.contains(&pid) {
                ids.push(pid);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    ids.into_iter()
        .map(|pid| {
            let ppid = all
                .iter()
                .find(|(p, _)| *p == pid)
                .map(|(_, pp)| *pp)
                .unwrap_or(0);
            (pid, ppid)
        })
        .collect()
}

fn carries_login_flag(cmdline: &[u8]) -> bool {
    contains_bytes(cmdline, b"-L\0")
        || contains_bytes(cmdline, b"--login\0")
        || contains_bytes(cmdline, b"--login=")
}

fn observe_scrub(pid: u32) -> Option<bool> {
    let maps = std::fs::read(format!("/proc/{pid}/maps")).unwrap_or_default();
    let env = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    // A leaf may exit between these procfs reads. Its disappearance is not
    // an observation of a new, unconfirmed image.
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if cmd.is_empty() {
        return None;
    }
    Some(contains_bytes(&maps, b"argv-scrub.so") && contains_bytes(&env, b"MELBA_LOGIN_APPLIED=1"))
}

fn kill_pids(pids: &[u32]) {
    for &pid in pids {
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
}

struct WatchResult {
    leaked: bool,
    killed_unconfirmed: bool,
    observed_confirmed: bool,
}

fn watch_login_tree(root: u32, cred: String, stop: Arc<AtomicBool>) -> WatchResult {
    let mut unready_since: HashMap<u32, Instant> = HashMap::new();
    let mut terminal_ready = HashMap::new();
    let needle = cred.into_bytes();
    while !stop.load(Ordering::Relaxed) {
        let rows = proc_tree(root);
        // A launcher's confirmation cannot stand in for its child's image.
        for &(_, parent) in &rows {
            terminal_ready.remove(&parent);
        }
        let pids: Vec<u32> = rows.iter().map(|(pid, _)| *pid).collect();
        let mut leaked = false;
        let mut login_pids = Vec::new();
        for &pid in &pids {
            let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if contains_bytes(&cmd, &needle) {
                leaked = true;
                break;
            }
            if carries_login_flag(&cmd) {
                login_pids.push(pid);
            }
        }
        if leaked {
            kill_pids(&pids);
            return WatchResult {
                leaked: true,
                killed_unconfirmed: false,
                observed_confirmed: false,
            };
        }
        let leaves: Vec<u32> = login_pids
            .into_iter()
            .filter(|pid| {
                !rows
                    .iter()
                    .any(|&(child, ppid)| child != *pid && ppid == *pid)
            })
            .collect();
        let now = Instant::now();
        unready_since.retain(|pid, _| leaves.contains(pid));
        let mut stale = false;
        for pid in leaves {
            let Some(ready) = observe_scrub(pid) else {
                continue;
            };
            // Replace the result after exec; retain a departed terminal leaf's
            // failure so a confirmed waiting parent cannot overwrite it.
            terminal_ready.insert(pid, ready);
            if ready {
                unready_since.remove(&pid);
            } else {
                let since = unready_since.entry(pid).or_insert(now);
                if now.duration_since(*since) >= SCRUB_GRACE {
                    stale = true;
                }
            }
        }
        if stale {
            kill_pids(&pids);
            return WatchResult {
                leaked: false,
                killed_unconfirmed: true,
                observed_confirmed: false,
            };
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    WatchResult {
        leaked: false,
        killed_unconfirmed: false,
        observed_confirmed: !terminal_ready.is_empty()
            && terminal_ready.values().all(|ready| *ready),
    }
}

fn write_login_memfd(cred: &str) -> Result<std::fs::File, String> {
    use std::io::Write;
    use std::os::fd::FromRawFd;
    let name = std::ffi::CString::new("melba-login").map_err(|_| "memfd name".to_string())?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!("memfd_create: {}", std::io::Error::last_os_error()));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(cred.as_bytes())
        .map_err(|e| format!("write login credential: {e}"))?;
    Ok(file)
}

fn inherit_login_fd(cred_fd: i32) -> std::io::Result<()> {
    if unsafe { libc::dup2(cred_fd, MELBA_LOGIN_FD) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // dup2(fd, fd) leaves FD_CLOEXEC unchanged, unlike a duplication to another fd.
    if unsafe { libc::fcntl(MELBA_LOGIN_FD, libc::F_SETFD, 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if cred_fd != MELBA_LOGIN_FD {
        unsafe { libc::close(cred_fd) };
    }
    Ok(())
}

/// One-time provisioning: run the service in the foreground with Apple ID
/// credentials so the device/account database is created. Afterwards the
/// managed service starts without login args.
pub fn login(user: &str, pass: &str) -> Result<String, String> {
    use std::os::fd::AsRawFd;
    if pass.trim().is_empty() {
        // `-L user:` exits 0 without ever attempting auth — fail fast instead
        // of letting the user believe login ran
        return Err("empty password — nothing would be authenticated".into());
    }
    if user.contains('\0') || pass.contains('\0') {
        return Err("login credential contains NUL".into());
    }
    // Upstream's only credential option is `-L`/`--login username:password`
    // (wrapper.ggo). Putting that string on argv makes it world-readable in
    // /proc/<pid>/cmdline until something scrubs it. Spawn with `-L @` and
    // the bytes in a memfd; argv-scrub.so repoints the pointer in each image
    // and puts `@` back on every execve.
    let cred = format!("{user}:{pass}");
    if cred.len() > MAX_LOGIN_CRED {
        return Err("login credential is too long".into());
    }
    let _lease = session_lock()?;
    let bin = binary()
        .ok_or_else(|| "wrapper not installed — run `melbad wrapper setup` first".to_string())?;
    let mut cmd = Command::new(&bin);
    let d = dir().ok_or("no XDG_DATA_HOME or HOME")?;
    let lib64 = d.join("rootfs/system/lib64");
    if !argv_scrub_ready(&lib64) {
        return Err(
            "argv-scrub.so is missing or out of date — refusing to start login. \
             Install `cc` and re-run `melbad wrapper setup`"
                .into(),
        );
    }
    let credential = write_login_memfd(&cred)?;
    let cred_fd = credential.as_raw_fd();
    cmd.env("LD_PRELOAD", "argv-scrub.so")
        .env("LD_LIBRARY_PATH", &lib64)
        .env("MELBA_LOGIN_APPLIED", "0")
        .env_remove("MELBA_WRAPPER_PASSWORD");
    // pre_exec forces fork/exec; only async-signal-safe descriptor operations run here.
    unsafe {
        cmd.pre_exec(move || inherit_login_fd(cred_fd));
    }
    let spawned = crate::auth_gui::spawn_provisioning(
        cmd.current_dir(&d)
            .args(["-L", "@", "-F"])
            .stdin(Stdio::null()), // stdin belongs to the 2FA reader below
    );
    drop(credential);
    let mut child = spawned?;
    // -F: the wrapper polls <base>/2fa.txt for a 2FA code instead of a
    // dialog a headless launcher can't show. The chroot resolves the
    // absolute -B against its own root, so the effective file lives at
    // the doubled path — write to both candidates.
    let base = d.join("rootfs/data/data/com.apple.android.music/files");
    let mut candidates = vec![base.join("2fa.txt")];
    // inside the chroot the absolute -B is resolved against the rootfs —
    // host-side it lands at rootfs/<abs -B>, i.e. the doubled path
    if let Ok(rel) = base.strip_prefix("/") {
        candidates.push(d.join("rootfs").join(rel).join("2fa.txt"));
    }
    eprintln!(
        "if a trusted device shows a 6-digit code, paste it here and press Enter\n\
         (or write it yourself to {})",
        candidates[0].display()
    );
    let targets: Vec<PathBuf> = candidates;
    std::thread::spawn(move || {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            let code = line.trim();
            if !code.is_empty() {
                for t in &targets {
                    let _ = std::fs::write(t, code);
                }
                eprintln!("2FA code delivered to {}", targets[0].display());
            }
        }
    });
    let root = child.id();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_watch = Arc::clone(&stop);
    let watch = std::thread::spawn(move || watch_login_tree(root, cred, stop_watch));
    let waited = child.wait();
    stop.store(true, Ordering::SeqCst);
    let watched = watch.join().unwrap_or(WatchResult {
        leaked: false,
        killed_unconfirmed: false,
        observed_confirmed: false,
    });
    if watched.leaked {
        return Err(
            "login aborted: the Apple ID credential was visible on a process \
             command line; the child was killed"
                .into(),
        );
    }
    if watched.killed_unconfirmed {
        return Err(
            "login aborted: argv-scrub.so did not take effect; the child was killed".into(),
        );
    }
    let st = waited.map_err(|e| format!("wait {}: {e}", bin.display()))?;
    // the wrapper exits 0 even on `[!] login failed` — don't claim success
    // unless the shim confirmed, and otherwise point at the observable state
    if !watched.observed_confirmed {
        return Err(format!(
            "login aborted: argv-scrub.so did not take effect, and the process exited with {st}"
        ));
    }
    if st.success() {
        Ok(
            "provisioning process exited successfully; monitoring is sampled, \
            so terminal-image delivery and account authentication are not verified. \
            Check `melbad wrapper status` / `melbad doctor`; rejected credentials \
            or 2FA may appear as `login failed` in the wrapper log."
                .into(),
        )
    } else {
        Err(format!("login pass exited with {st}"))
    }
}

pub fn setup(apk: Option<&Path>) -> Result<String, String> {
    let _lease = session_lock()?;
    crate::auth_gui::invalidate_binding()?;
    const SCRIPT: &str = include_str!("../packaging/install-wrapper.sh");
    const SCRUB_SRC: &str = include_str!("../packaging/argv-scrub.c");
    let d = dir().ok_or("no XDG_DATA_HOME or HOME")?;
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    // install-wrapper.sh resolves argv-scrub.c relative to $0 — stage
    // both in one hidden dir so the sibling lookup works.
    let sdir = d.join(".install");
    std::fs::create_dir_all(&sdir).map_err(|e| e.to_string())?;
    let script = sdir.join("install-wrapper.sh");
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    std::fs::write(sdir.join("argv-scrub.c"), SCRUB_SRC).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("bash");
    cmd.arg(&script).arg("--dir").arg(&d);
    if let Some(a) = apk {
        cmd.arg("--apk").arg(a);
    }
    let st = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err(format!("install-wrapper.sh: {st}"));
    }
    // staging dir was only needed for the sibling script lookup —
    // on success the install is complete and it is an artifact
    let _ = std::fs::remove_dir_all(&sdir);
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_descriptor_remains_inheritable_at_its_reserved_number() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "test -e /proc/self/fd/250"]);
        unsafe {
            command.pre_exec(|| {
                // Set up the rare memfd_create == reserved-fd case in the child only.
                let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
                if fd < 0 || libc::dup2(fd, MELBA_LOGIN_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if fd != MELBA_LOGIN_FD {
                    libc::close(fd);
                }
                if libc::fcntl(MELBA_LOGIN_FD, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                inherit_login_fd(MELBA_LOGIN_FD)
            });
        }
        assert!(command.status().unwrap().success());
    }

    #[test]
    fn check_wrapper_parses_forms() {
        // unroutable but well-formed host:port → connection error names the host
        let e = check_wrapper("tcp://127.0.0.1:1").unwrap_err();
        assert!(e.contains("127.0.0.1:1"), "{e}");
        let e = check_wrapper("http://localhost:1/path").unwrap_err();
        assert!(e.contains("localhost:1"), "{e}");
    }

    #[test]
    fn early_exit_latch_short_circuits_try_ready() {
        // a latched early-exit must fail fast with the retry hint instead
        // of spawning the service and waiting its whole startup window
        EARLY_EXIT.store(true, Ordering::SeqCst);
        let e = try_ready().unwrap_err();
        EARLY_EXIT.store(false, Ordering::SeqCst);
        assert!(e.contains("exited early"), "{e}");
        assert!(e.contains("wrapper start"), "{e}");
    }

    #[test]
    fn login_rejects_empty_password() {
        let e = login("user@example.com", "  ").unwrap_err();
        assert!(e.contains("empty password"), "{e}");
    }

    #[test]
    fn argv_scrub_must_carry_the_memfd_marker() {
        let root = std::env::temp_dir().join(format!("melba-scrub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib64 = root.join("lib64");
        std::fs::create_dir_all(&lib64).unwrap();
        assert!(!argv_scrub_ready(&lib64));
        std::fs::write(lib64.join("argv-scrub.so"), b"").unwrap();
        assert!(!argv_scrub_ready(&lib64));
        std::fs::write(lib64.join("argv-scrub.so"), b"\x7fELF").unwrap();
        assert!(!argv_scrub_ready(&lib64));
        std::fs::write(lib64.join("argv-scrub.so"), b"pre melba-login-fd-v2 post").unwrap();
        assert!(argv_scrub_ready(&lib64));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn env_password_is_taken_and_cleared() {
        let key = std::ffi::CString::new("MELBA_WRAPPER_PASSWORD").unwrap();
        let val = std::ffi::CString::new("env-secret").unwrap();
        unsafe { libc::setenv(key.as_ptr(), val.as_ptr(), 1) };
        let got = read_login_password("user@example.com").unwrap();
        assert_eq!(got, "env-secret");
        assert!(std::env::var_os("MELBA_WRAPPER_PASSWORD").is_none());

        let empty = std::ffi::CString::new("").unwrap();
        unsafe { libc::setenv(key.as_ptr(), empty.as_ptr(), 1) };
        let err = read_login_password("user@example.com").unwrap_err();
        assert!(err.contains("empty"), "{err}");
        assert!(std::env::var_os("MELBA_WRAPPER_PASSWORD").is_none());
    }
}
