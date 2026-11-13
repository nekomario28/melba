//! Command ingress never waits on source I/O. It interrupts an operation, but
//! only the playback owner consumes commands and changes queue / ALSA state.
use super::{Cmd, Shared};
use melba_cdm::operation::Operation;
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SkipDirection {
    Forward,
    Backward,
}
impl SkipDirection {
    pub(super) fn of(command: &Cmd) -> Option<Self> {
        match command {
            Cmd::Next => Some(Self::Forward),
            Cmd::Prev => Some(Self::Backward),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Routing {
    issued: u64,
    consumed: u64,
    active: Option<Operation>,
}
pub(super) struct Commands {
    receiver: Receiver<Cmd>,
    pending: RefCell<Option<Cmd>>,
    routing: Arc<Mutex<Routing>>,
    closed: Arc<AtomicBool>,
    ingress: Option<JoinHandle<()>>,
}
fn supersedes(command: &Cmd) -> bool {
    SkipDirection::of(command).is_some()
        || matches!(
            command,
            Cmd::Stop
                | Cmd::Quit
                | Cmd::LoadTrack(_)
                | Cmd::PlayQueued { .. }
                | Cmd::LoadQueue(_)
                | Cmd::OpenUri(_)
        )
}
impl Commands {
    #[cfg(test)]
    pub(super) fn new(receiver: Receiver<Cmd>) -> Self {
        Self::plain(receiver)
    }
    fn plain(receiver: Receiver<Cmd>) -> Self {
        Self {
            receiver,
            pending: RefCell::new(None),
            routing: Arc::new(Mutex::new(Routing::default())),
            closed: Arc::new(AtomicBool::new(false)),
            ingress: None,
        }
    }
    pub(super) fn routed(shared: Arc<Shared>, receiver: Receiver<Cmd>) -> Self {
        let (sender, forwarded) = mpsc::channel();
        let mut commands = Self::plain(forwarded);
        let routing = commands.routing.clone();
        let closed = commands.closed.clone();
        commands.ingress = Some(std::thread::spawn(move || {
            while !closed.load(Ordering::Acquire) {
                let command = match receiver.recv_timeout(Duration::from_millis(200)) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let changes_source = supersedes(&command);
                let valid = match &command {
                    Cmd::LoadTrack(index) => super::goto_valid(&shared, *index),
                    Cmd::LoadQueue(load) => load.index < load.paths.len(),
                    Cmd::PlayQueued { row, uri } => {
                        super::queued_selection_valid(&shared, *row, uri)
                    }
                    _ => true,
                };
                let quit = matches!(command, Cmd::Quit);
                let mut state = routing.lock().unwrap_or_else(|e| e.into_inner());
                if changes_source {
                    state.issued += 1;
                }
                if sender.send(command).is_err() {
                    break;
                }
                // Enqueue and cancellation are atomic with begin(): an operation
                // created after this send must notice the unconsumed selection.
                if changes_source && valid {
                    if let Some(operation) = &state.active {
                        operation.cancel();
                    }
                }
                drop(state);
                if quit {
                    break;
                }
            }
            if let Some(operation) = &routing.lock().unwrap_or_else(|e| e.into_inner()).active {
                operation.cancel();
            }
        }));
        commands
    }
    pub(super) fn begin(&self) -> Operation {
        let operation = Operation::default();
        let mut routing = self.routing.lock().unwrap_or_else(|e| e.into_inner());
        if routing.issued > routing.consumed {
            operation.cancel();
        }
        let previous = routing.active.replace(operation.clone());
        drop(routing);
        if let Some(previous) = previous {
            previous.cancel();
        }
        operation
    }
    fn delivered(&self, command: Cmd) -> Cmd {
        if supersedes(&command) {
            self.routing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .consumed += 1;
        }
        command
    }
    fn raw_try_recv(&self) -> Result<Cmd, mpsc::TryRecvError> {
        if let Some(command) = self.pending.borrow_mut().take() {
            return Ok(command);
        }
        self.receiver.try_recv()
    }
    pub(super) fn recv(&self) -> Result<Cmd, mpsc::RecvError> {
        let command = if let Some(command) = self.pending.borrow_mut().take() {
            Ok(command)
        } else {
            self.receiver.recv()
        }?;
        Ok(self.delivered(command))
    }
    pub(super) fn try_recv(&self) -> Result<Cmd, mpsc::TryRecvError> {
        self.raw_try_recv().map(|command| self.delivered(command))
    }
    pub(super) fn recv_timeout(&self, timeout: Duration) -> Result<Cmd, mpsc::RecvTimeoutError> {
        let command = if let Some(command) = self.pending.borrow_mut().take() {
            Ok(command)
        } else {
            self.receiver.recv_timeout(timeout)
        }?;
        Ok(self.delivered(command))
    }
    pub(super) fn has_pending(&self) -> bool {
        match self.raw_try_recv() {
            Ok(command) => {
                self.pending.replace(Some(command));
                true
            }
            Err(_) => false,
        }
    }
    pub(super) fn take_skip(&self) -> Option<SkipDirection> {
        match self.raw_try_recv() {
            Ok(command) => match SkipDirection::of(&command) {
                Some(direction) => {
                    self.delivered(command);
                    Some(direction)
                }
                None => {
                    self.pending.replace(Some(command));
                    None
                }
            },
            Err(_) => None,
        }
    }
}
impl Drop for Commands {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(operation) = &self
            .routing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
        {
            operation.cancel();
        }
        if let Some(ingress) = self.ingress.take() {
            let _ = ingress.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn issued(commands: &Commands, expected: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while commands.routing.lock().unwrap().issued < expected {
            assert!(
                std::time::Instant::now() < deadline,
                "ingress did not receive controls"
            );
            std::thread::yield_now();
        }
    }
    #[test]
    fn queued_selection_cannot_be_missed_when_a_new_operation_begins() {
        let (sender, receiver) = mpsc::channel();
        let commands = Commands::routed(Shared::new(), receiver);
        let first = commands.begin();
        sender.send(Cmd::Next).unwrap();
        issued(&commands, 1);
        assert!(first.is_cancelled());
        assert!(
            commands.begin().is_cancelled(),
            "selection is still unconsumed"
        );
        assert_eq!(commands.take_skip(), Some(SkipDirection::Forward));
        assert!(
            !commands.begin().is_cancelled(),
            "consumed selection starts fresh work"
        );
        sender.send(Cmd::Quit).unwrap();
    }
    #[test]
    fn lookahead_does_not_acknowledge_a_command_barrier_twice() {
        let (sender, receiver) = mpsc::channel();
        let commands = Commands::routed(Shared::new(), receiver);
        for command in [Cmd::Next, Cmd::Stop, Cmd::Next] {
            sender.send(command).unwrap();
        }
        issued(&commands, 3);
        assert_eq!(commands.take_skip(), Some(SkipDirection::Forward));
        assert_eq!(commands.take_skip(), None);
        assert_eq!(commands.routing.lock().unwrap().consumed, 1);
        assert!(commands.begin().is_cancelled());
        assert!(matches!(commands.recv().unwrap(), Cmd::Stop));
        assert_eq!(commands.take_skip(), Some(SkipDirection::Forward));
        assert_eq!(commands.routing.lock().unwrap().consumed, 3);
        assert!(!commands.begin().is_cancelled());
        sender.send(Cmd::Quit).unwrap();
    }
}
