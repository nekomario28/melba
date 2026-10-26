//! One lifecycle owner for locally spawned native process groups.
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SUPERVISOR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/melba-supervisor"));
/// The supervisor's own descendant sweep is bounded to a second; the owner
/// waits longer, so escalation never cuts a slow sweep short.
const ESCALATION_WINDOW: Duration = Duration::from_millis(2000);

/// Staging must not follow a symlinked ancestor: the rootfs tree belongs to
/// us, so a symlink above the destination means the path was redirected and
/// the private permissions/staging would land outside the intended tree.
/// `private_dir` only refuses a symlinked final component.
fn reject_symlinked_ancestors(path: &Path) -> Result<(), String> {
    let mut walked = PathBuf::new();
    for component in path.components() {
        if walked == path {
            break;
        }
        walked.push(component);
        match std::fs::symlink_metadata(&walked) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("cannot stage local process support".into());
            }
            Ok(_) => {}
            // The rest of the tree is created by us in the same call.
            Err(_) => break,
        }
    }
    Ok(())
}

/// Cache only the exact embedded artifact, privately and atomically. Shared
/// by host supervision and the authentication presentation library.
pub(crate) fn stage(
    directory: &Path,
    label: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<PathBuf, String> {
    use sha2::Digest;
    reject_symlinked_ancestors(directory)?;
    crate::auth::private_dir(directory)?;
    let digest = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mode = if executable { 0o700 } else { 0o600 };
    let destination = directory.join(format!(
        "{label}-{digest}{}",
        if executable { "" } else { ".so" }
    ));
    let cached_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&destination)
        .ok()
        .filter(|file| {
            file.metadata().is_ok_and(|m| {
                m.is_file()
                    && m.len() == bytes.len() as u64
                    && m.uid() == unsafe { libc::geteuid() }
                    && m.mode() & 0o777 == mode
            })
        });
    if let Some(file) = cached_file {
        let mut cached = Vec::new();
        if file
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut cached)
            .is_ok()
            && cached == bytes
        {
            return Ok(destination);
        }
    }
    let temporary = directory.join(format!(".{label}-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|_| "cannot stage local process support")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "cannot stage local process support")?;
        std::fs::rename(&temporary, &destination)
            .map_err(|_| "cannot stage local process support")?;
        Ok(destination)
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

/// An ignored `SIGCHLD` (or `SA_NOCLDWAIT`) makes the kernel reap our children
/// behind our back, which removes the PID reservation every signal below
/// depends on. Refuse to supervise rather than risk signalling a recycled
/// process group.
fn reaping_is_available() -> bool {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } != 0 {
        return false;
    }
    action.sa_sigaction != libc::SIG_IGN && action.sa_flags & libc::SA_NOCLDWAIT == 0
}

/// CLI `wrapper start` is explicitly detached; playback and authentication
/// instead end with the owner process, including when its creating thread ends.
pub(crate) fn command(directory: &Path, program: &Path, detached: bool) -> Result<Command, String> {
    let supervisor = stage(directory, "melba-supervisor", SUPERVISOR, true)?;
    let mut command = Command::new(supervisor);
    command
        .arg(if detached { 0 } else { std::process::id() }.to_string())
        .arg(program);
    Ok(command)
}

pub(crate) struct ProcessGroup {
    child: Child,
    /// Set once the kernel says this child is already reaped: from then on the
    /// numeric PID is no longer ours to signal.
    lost: AtomicBool,
}
impl ProcessGroup {
    pub(crate) fn spawn(command: &mut Command) -> Result<Self, String> {
        if !reaping_is_available() {
            return Err("cannot supervise a child while SIGCHLD is ignored".into());
        }
        // The supervisor leads the group and the worker stays in it, so this
        // owner only ever signals a PID it still holds unreaped. Nothing here
        // is derived from a port scan, /proc search or a stale numeric id.
        // The staged supervisor can be replaced by another thread in the
        // same test process between rename and exec. A transient ETXTBSY is
        // not a lifetime failure; retry only that errno with a bounded wait.
        let mut attempts = 0;
        let child = loop {
            match command.process_group(0).spawn() {
                Ok(child) => break child,
                Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempts < 100 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        Ok(Self {
            child,
            lost: AtomicBool::new(false),
        })
    }
    /// Reap without blocking. Used when the child could not be signalled, so
    /// the caller must not wait on a process that may outlive it.
    fn reap_bounded(&self, window: Duration) {
        let deadline = Instant::now() + window;
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG,
                )
            };
            if result != 0 || unsafe { info.si_pid() } != 0 || Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// Observe exit without reaping: keep the PID reserved until the final
    /// process-group signal, so cleanup cannot target a recycled PID.
    pub(crate) fn status(&self) -> Result<Option<bool>, String> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            // ECHILD means something outside this handle already reaped the
            // child, so the PID reservation is gone and cleanup must not name
            // it again.
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                self.lost.store(true, Ordering::SeqCst);
                return Ok(Some(false));
            }
            return Err("cannot inspect owned child".into());
        }
        Ok(if unsafe { info.si_pid() } == 0 {
            None
        } else {
            Some(info.si_code == libc::CLD_EXITED && unsafe { info.si_status() } == 0)
        })
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Never signal a number the kernel may have recycled. `status()` first:
        // a live or unreaped (zombie) supervisor still holds its PID, and only
        // ECHILD means that guarantee is gone. An ignored SIGCHLD reaps the
        // child behind our back at any moment, which removes that reservation,
        // so nothing is signalled in that state. Melba never changes the
        // disposition after start (spawn refuses it); an embedder that did
        // gets a leak rather than a signal aimed at an unrelated group.
        let leader = self.child.id() as libc::pid_t;
        if reaping_is_available()
            && !self.lost.load(Ordering::SeqCst)
            && self.status().is_ok()
            && !self.lost.load(Ordering::SeqCst)
        {
            // SIGCONT first: a supervisor that was stopped or descheduled can
            // then run its own cleanup, which also ends descendants that left
            // this process group. SIGTERM is not delivered to a stopped
            // process, so the owner cannot rely on the signal alone.
            unsafe {
                libc::kill(-leader, libc::SIGCONT);
                libc::kill(-leader, libc::SIGTERM);
            }
            let deadline = Instant::now() + ESCALATION_WINDOW;
            while Instant::now() < deadline {
                if self.status().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !self.lost.load(Ordering::SeqCst) {
                // Still ours: the whole group (supervisor, worker, and every
                // descendant that stayed in it) ends with it.
                unsafe {
                    libc::kill(-leader, libc::SIGKILL);
                }
            }
            let _ = self.child.wait();
        } else if self.lost.load(Ordering::SeqCst) {
            // Already reaped elsewhere; nothing left to collect.
            let _ = self.child.wait();
        } else {
            // Unsignalled and possibly still running: the supervisor waits for
            // this process to die first, so blocking here would deadlock.
            self.reap_bounded(Duration::from_millis(200));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `execv` does not search PATH, so the fixture needs a real path.
    fn python() -> PathBuf {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("python3"))
                    .find(|candidate| candidate.is_file())
            })
            .expect("python3 is required by the process-lifetime regressions")
    }

    fn service(folder: &Path, extra: &[&str]) -> Command {
        let mut command = super::command(folder, &python(), false).unwrap();
        command
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/process_owner.py"
            ))
            .arg("--service")
            .arg(folder)
            .args(extra);
        command
    }

    fn wait_for(folder: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !folder.join("pids").exists() {
            assert!(Instant::now() < deadline, "fixture never started");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn pids(folder: &Path) -> Vec<libc::pid_t> {
        std::fs::read_to_string(folder.join("pids"))
            .unwrap()
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect()
    }

    /// The fixture processes are not this process's children, so liveness
    /// comes from /proc rather than waitid (which would answer ECHILD).
    fn alive(pid: libc::pid_t) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .is_some_and(|state| state != "Z"),
            Err(_) => false,
        }
    }

    fn gone(folder: &Path) {
        let owned = pids(folder);
        let deadline = Instant::now() + Duration::from_secs(2);
        while owned.iter().copied().any(alive) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !owned.iter().copied().any(alive),
            "owned process survived cleanup: {owned:?}"
        );
    }

    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("melba-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A stopped supervisor cannot process SIGTERM until it runs again, and a
    /// descendant that left its process group is outside any group signal.
    /// Only resuming the supervisor ends that descendant.
    #[test]
    fn a_stopped_supervisor_is_resumed_to_end_an_escaped_descendant() {
        let folder = folder("escaped-stopped");
        let mut command = service(&folder, &["--escape"]);
        let group = ProcessGroup::spawn(&mut command).unwrap();
        wait_for(&folder);
        assert_eq!(pids(&folder).len(), 2, "worker and its escaped child");
        unsafe { libc::kill(group.child.id() as libc::pid_t, libc::SIGSTOP) };
        assert_eq!(group.status().unwrap(), None);
        drop(group);
        gone(&folder);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A supervisor that ignores SIGTERM is escalated to a group kill, which
    /// still ends the worker because the worker never left its group.
    #[test]
    fn an_unkillable_supervisor_is_escalated_for_the_whole_group() {
        let folder = folder("wedged");
        let mut command = service(&folder, &[]);
        let group = ProcessGroup::spawn(&mut command).unwrap();
        wait_for(&folder);
        let leader = group.child.id() as libc::pid_t;
        // SIGSTOP cannot be handled: the escalation must finish the job.
        unsafe { libc::kill(leader, libc::SIGSTOP) };
        // Sabotage the resume path so only the group kill can succeed.
        unsafe { libc::kill(leader, libc::SIGKILL) };
        assert!(group.status().is_ok());
        drop(group);
        gone(&folder);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The ignored disposition must really be the one under test.
    fn ignored_sigchld_scenario(folder: &Path) {
        // Spawn first, then lose the reservation: an embedder that changes
        // SIGCHLD after start must not make cleanup signal a recycled PID.
        let mut command = service(folder, &[]);
        let group = ProcessGroup::spawn(&mut command).unwrap();
        wait_for(folder);
        unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
        unsafe { libc::kill(-(group.child.id() as libc::pid_t), libc::SIGKILL) };
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(group.status().unwrap(), Some(false));
        assert!(group.lost.load(Ordering::SeqCst));
        drop(group);
        gone(folder);
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A supervisor PID the kernel already reaped is never signalled again.
    #[test]
    fn already_reaped_supervisor_is_not_signalled_again() {
        let folder = folder("reaped");
        // fork so the SIGCHLD disposition stays out of the test runner; a
        // panic in the forked child never reaches the runner, so the verdict
        // travels back as its exit status.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "cannot fork");
        if pid == 0 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ignored_sigchld_scenario(&folder);
            }));
            unsafe { libc::_exit(if outcome.is_ok() { 0 } else { 1 }) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let _ = std::fs::remove_dir_all(&folder);
        assert!(libc::WIFEXITED(status), "scenario did not exit: {status}");
        assert_eq!(libc::WEXITSTATUS(status), 0, "scenario failed");
    }

    /// Supervising without a PID reservation is refused instead of risking an
    /// unrelated process group.
    #[test]
    fn an_ignored_sigchld_refuses_to_supervise() {
        let folder = folder("ignored");
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "cannot fork");
        if pid == 0 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
                let mut command = service(&folder, &[]);
                let refused = ProcessGroup::spawn(&mut command);
                assert!(refused.is_err(), "supervising without reaping was accepted");
                assert!(!folder.join("pids").exists(), "a child was left running");
            }));
            unsafe { libc::_exit(if outcome.is_ok() { 0 } else { 1 }) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let _ = std::fs::remove_dir_all(&folder);
        assert!(libc::WIFEXITED(status), "scenario did not exit: {status}");
        assert_eq!(libc::WEXITSTATUS(status), 0, "scenario failed");
    }

    /// A symlink in a staging ancestor would redirect private permissions and
    /// the staged artifact outside the tree we own.
    #[test]
    fn symlinked_staging_ancestor_is_refused() {
        let folder = folder("symlink");
        let outside = folder.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(folder.join("tree")).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("tree/rootfs")).unwrap();
        let target = folder.join("tree/rootfs/system/lib64");
        let result = super::stage(&target, "melba-supervisor", super::SUPERVISOR, true);
        assert!(result.is_err(), "symlinked staging ancestor was accepted");
        assert!(
            !outside.join("system").exists(),
            "staging escaped through the symlink"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn isolated_owner_death_reaps_workers_and_releases_the_session_lock() {
        let folder =
            std::env::temp_dir().join(format!("melba-supervisor-test-{}", std::process::id()));
        let supervisor =
            super::stage(&folder, "melba-supervisor", super::SUPERVISOR, true).unwrap();
        let result = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/process_owner.py"
            ))
            .arg(supervisor)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(folder);
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
