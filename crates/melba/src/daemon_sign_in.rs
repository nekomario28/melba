//! Own the CLI and its private pipes off the UI thread.
use melba_auth::{Event, Input, MAX_PACKET};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

pub(crate) struct SignIn {
    pub id: u64,
    cancel: Arc<AtomicBool>,
    input: mpsc::SyncSender<Input>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for SignIn {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl SignIn {
    pub fn cancel(mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
    pub fn send(&self, input: Input) -> bool {
        self.input.try_send(input).is_ok()
    }
    pub fn start(
        id: u64,
        restore: bool,
        mut command: Command,
        output: tokio::sync::mpsc::UnboundedSender<(u64, Event)>,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let (input, rx) = mpsc::sync_channel::<Input>(8);
        let thread = std::thread::spawn(move || {
            let failed = || Event::Failed {
                message: "Sign-in did not complete. Check the installed melbad and try again."
                    .into(),
            };
            let result = (|| {
                let mut child = command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|_| ())?;
                let mut stdin = child.stdin.take();
                let mut stdout = child.stdout.take().ok_or(())?;
                for fd in [stdout.as_raw_fd(), stdin.as_ref().ok_or(())?.as_raw_fd()] {
                    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                    if flags < 0
                        || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                    {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(());
                    }
                }
                let deadline = Instant::now() + Duration::from_secs(620);
                let mut line = Vec::new();
                let mut terminal = None;
                let mut committed = false;
                let mut pending = Vec::new();
                let mut position = 0;
                let mut committing = false;
                let result = (|| {
                    loop {
                        if flag.load(Ordering::Acquire) || Instant::now() > deadline {
                            return Err(());
                        }
                        if pending.is_empty()
                            && let Ok(input) = rx.try_recv()
                        {
                            committing = matches!(input, Input::Commit);
                            pending = serde_json::to_vec(&input).map_err(|_| ())?;
                            pending.push(b'\n');
                            if pending.len() > MAX_PACKET {
                                return Err(());
                            }
                            position = 0;
                        }
                        if !pending.is_empty() {
                            match stdin.as_mut().ok_or(())?.write(&pending[position..]) {
                                Ok(0) => return Err(()),
                                Ok(n) => {
                                    position += n;
                                    if position == pending.len() {
                                        committed |= committing;
                                        melba_auth::erase(&mut pending);
                                        pending.clear();
                                    }
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(_) => return Err(()),
                            }
                        }
                        let mut chunk = [0u8; 4096];
                        match stdout.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                for byte in &chunk[..n] {
                                    if *byte == b'\n' {
                                        let event = serde_json::from_slice::<Event>(&line)
                                            .map_err(|_| ())?;
                                        line.clear();
                                        if terminal.is_some() {
                                            return Err(());
                                        }
                                        match event {
                                            Event::Complete if !restore && !committed => {
                                                return Err(());
                                            }
                                            Event::Complete
                                            | Event::SignedOut
                                            | Event::Failed { .. } => terminal = Some(event),
                                            event => {
                                                let _ = output.send((id, event));
                                            }
                                        }
                                    } else {
                                        if line.len() >= MAX_PACKET {
                                            return Err(());
                                        }
                                        line.push(*byte);
                                    }
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(_) => return Err(()),
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if !line.is_empty() {
                        return Err(());
                    }
                    Ok(())
                })();
                melba_auth::erase(&mut pending);
                // EOF/Cancel closes the parent's input. melbad then tears down
                // its OWN native group, including while Apple validation runs.
                drop(stdin.take());
                let end = Instant::now() + Duration::from_secs(3);
                let status = loop {
                    if let Some(status) = child.try_wait().map_err(|_| ())? {
                        break Some(status);
                    }
                    if Instant::now() > end {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                result?;
                match terminal {
                    Some(event @ Event::Failed { .. }) => Ok(event),
                    Some(event) if status.is_some_and(|s| s.success()) => Ok(event),
                    _ => Err(()),
                }
            })();
            if !flag.load(Ordering::Acquire) {
                let _ = output.send((id, result.unwrap_or_else(|_| failed())));
            }
        });
        Self {
            id,
            cancel,
            input,
            thread: Some(thread),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peer(code: &str) -> (SignIn, tokio::sync::mpsc::UnboundedReceiver<(u64, Event)>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut command = Command::new("python3");
        command.args(["-c", code]);
        (SignIn::start(7, false, command, tx), rx)
    }
    #[test]
    fn completion_requires_commit_and_successful_owned_exit() {
        for code in [
            "print('{\"type\":\"complete\"}',flush=True)",
            "import sys; print('{\"type\":\"signed_out\"}',flush=True); sys.exit(1)",
            "print('{\"type\":\"signed_out\"}\\n{\"type\":\"checking\"}',flush=True)",
        ] {
            let (job, mut rx) = peer(code);
            assert!(matches!(
                rx.blocking_recv(),
                Some((7, Event::Failed { .. }))
            ));
            job.cancel();
        }
    }
    #[test]
    fn commit_is_forwarded_before_completion() {
        let (job, mut rx) = peer(
            "import json,sys; print('{\"type\":\"ready_to_commit\"}',flush=True); assert json.loads(sys.stdin.readline())=={'type':'commit'}; print('{\"type\":\"complete\"}',flush=True)",
        );
        assert!(matches!(
            rx.blocking_recv(),
            Some((7, Event::ReadyToCommit))
        ));
        assert!(job.send(Input::Commit));
        assert!(matches!(rx.blocking_recv(), Some((7, Event::Complete))));
        job.cancel();
    }
    #[test]
    fn cancellation_interrupts_backpressured_secret_input() {
        let (job, mut rx) =
            peer("import time; print('{\"type\":\"checking\"}',flush=True); time.sleep(60)");
        assert!(matches!(rx.blocking_recv(), Some((7, Event::Checking))));
        assert!(job.send(Input::Credentials {
            id: 1,
            username: "fixture".to_string().into(),
            password: "x".repeat(50000).into()
        }));
        std::thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        job.cancel();
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn cancelled_attempt_cannot_deliver_late_completion() {
        let (job, mut rx) = peer(
            "import time; print('{\"type\":\"ready_to_commit\"}',flush=True); time.sleep(0.2); print('{\"type\":\"complete\"}',flush=True)",
        );
        assert!(matches!(
            rx.blocking_recv(),
            Some((7, Event::ReadyToCommit))
        ));
        job.cancel();
        assert!(rx.try_recv().is_err());
    }
}
