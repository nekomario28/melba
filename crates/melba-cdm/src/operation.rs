//! Interruptible source work. Cancellation owns only this operation's resources,
//! never a process-wide HTTP pool or another caller's sockets.
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};

type Interrupt = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct Resources {
    agents: HashMap<usize, ureq::Agent>,
    interrupts: HashMap<usize, Interrupt>,
    next: usize,
}
#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    resources: Mutex<Resources>,
}
#[derive(Clone, Default)]
pub struct Operation(Arc<Inner>);
thread_local! { static CURRENT: RefCell<Option<Operation>> = const { RefCell::new(None) }; }

/// A thread-local guard must be dropped on the thread that entered it.
/// ```compile_fail
/// let operation = melba_cdm::operation::Operation::default();
/// let scope = operation.enter();
/// std::thread::spawn(move || drop(scope));
/// ```
pub struct Scope {
    previous: Option<Operation>,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| slot.replace(self.previous.take()));
    }
}
pub struct Registration {
    owner: Weak<Inner>,
    key: usize,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner
                .resources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .interrupts
                .remove(&self.key);
        }
    }
}
impl Operation {
    pub fn enter(&self) -> Scope {
        Scope {
            previous: CURRENT.with(|slot| slot.replace(Some(self.clone()))),
            _thread_bound: std::marker::PhantomData,
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::other("source operation cancelled"))
        } else {
            Ok(())
        }
    }
    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let (agents, interrupts) = {
            let mut resources = self.0.resources.lock().unwrap_or_else(|e| e.into_inner());
            (
                std::mem::take(&mut resources.agents),
                std::mem::take(&mut resources.interrupts),
            )
        };
        for (_, agent) in agents {
            agent.shutdown();
        }
        for (_, interrupt) in interrupts {
            interrupt();
        }
    }
    pub fn register(&self, interrupt: impl FnOnce() + Send + 'static) -> Registration {
        let mut resources = self.0.resources.lock().unwrap_or_else(|e| e.into_inner());
        let key = resources.next;
        resources.next += 1;
        let mut interrupt: Option<Interrupt> = Some(Box::new(interrupt));
        if !self.is_cancelled() {
            resources.interrupts.insert(key, interrupt.take().unwrap());
        }
        drop(resources);
        if let Some(interrupt) = interrupt {
            interrupt();
        }
        Registration {
            owner: Arc::downgrade(&self.0),
            key,
        }
    }
    fn agent(&self, template: &'static ureq::Agent) -> ureq::Agent {
        let mut resources = self.0.resources.lock().unwrap_or_else(|e| e.into_inner());
        let agent = resources
            .agents
            .entry(template as *const _ as usize)
            .or_insert_with(|| ureq::Agent::new_with_config(template.config().clone()))
            .clone();
        let cancelled = self.is_cancelled();
        drop(resources);
        if cancelled {
            agent.clone().shutdown();
        }
        agent
    }
}
pub fn current() -> Option<Operation> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub fn check() -> io::Result<()> {
    current().map_or(Ok(()), |operation| operation.check())
}
pub fn agent(template: &'static ureq::Agent) -> ureq::Agent {
    current().map_or_else(|| template.clone(), |operation| operation.agent(template))
}
/// Register an individually configured agent (e.g. the uncredentialed wrapper).
pub fn register_agent(agent: ureq::Agent) -> (ureq::Agent, Option<Registration>) {
    let registration = current().map(|operation| {
        let interrupt = agent.clone();
        operation.register(move || interrupt.shutdown())
    });
    (agent, registration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };
    fn template() -> &'static ureq::Agent {
        static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
        AGENT.get_or_init(|| ureq::Agent::config_builder().proxy(None).build().into())
    }
    #[test]
    fn cancellation_interrupts_real_headers_and_body_without_releasing_the_server() {
        for body in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let (ready_tx, ready) = mpsc::channel();
            let (release_tx, release) = mpsc::channel();
            let (body_tx, body_started) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0; 1];
                    socket.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 4096);
                }
                if body {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                        .unwrap();
                }
                ready_tx.send(()).unwrap();
                let _ = release.recv_timeout(Duration::from_secs(5));
            });
            let operation = Operation::default();
            let worker_operation = operation.clone();
            let (done_tx, done) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let _scope = worker_operation.enter();
                let failed = match agent(template()).get(&url).call() {
                    Ok(mut response) if body => {
                        let mut reader = response.body_mut().as_reader();
                        let mut prefix = [0; 1];
                        reader.read_exact(&mut prefix).unwrap();
                        assert_eq!(prefix, *b"x");
                        body_tx.send(()).unwrap();
                        reader.read_to_end(&mut Vec::new()).is_err()
                    }
                    Ok(mut response) => response.body_mut().read_to_string().is_err(),
                    Err(_) => true,
                };
                done_tx.send(failed).unwrap();
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            if body {
                body_started.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            operation.cancel();
            let interrupted = done.recv_timeout(Duration::from_secs(2));
            release_tx.send(()).unwrap();
            server.join().unwrap();
            worker.join().unwrap();
            assert_eq!(
                interrupted,
                Ok(true),
                "a blocked socket must return before the server is released"
            );
        }
    }
    #[test]
    fn pools_are_reused_but_not_shared_with_other_operations() {
        assert!(std::ptr::eq(
            agent(template()).config(),
            template().config()
        ));
        let first = Operation::default();
        let _scope = first.enter();
        let a = agent(template());
        let b = agent(template());
        assert!(std::ptr::eq(a.config(), b.config()));
        assert!(!std::ptr::eq(a.config(), template().config()));
        let second = Operation::default();
        {
            let _nested = second.enter();
            assert!(!std::ptr::eq(a.config(), agent(template()).config()));
            first.cancel();
            assert!(!second.is_cancelled());
        }
        assert!(current().unwrap().is_cancelled());
    }
    #[test]
    fn deregistration_and_late_registration_are_safe() {
        let operation = Operation::default();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let add = || {
            let count = count.clone();
            move || {
                count.fetch_add(1, Ordering::SeqCst);
            }
        };
        let retired = operation.register(add());
        drop(retired);
        let _active = operation.register(add());
        operation.cancel();
        operation.cancel();
        let _late = operation.register(add());
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
