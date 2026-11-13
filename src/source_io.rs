//! The same interruptible reader serves tags and decoder probing. In particular,
//! a named pipe must not block in File::open before cancellation can reach it.
use melba_cdm::operation::{Operation, Registration};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek},
    os::{
        fd::AsRawFd,
        unix::{fs::OpenOptionsExt, net::UnixStream},
    },
    path::Path,
};
use symphonia::core::io::MediaSource;

pub(crate) struct SourceFile {
    file: File,
    regular: bool,
    operation: Option<Operation>,
    wake: Option<UnixStream>,
    _registration: Option<Registration>,
}
impl SourceFile {
    pub(crate) fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let operation = melba_cdm::operation::current();
        melba_cdm::operation::check()?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?;
        let regular = MediaSource::is_seekable(&file);
        let (wake, registration) = if let Some(operation) = &operation {
            let (reader, writer) = UnixStream::pair()?;
            let registration = operation.register(move || {
                let _ = writer.shutdown(std::net::Shutdown::Write);
            });
            (Some(reader), Some(registration))
        } else {
            (None, None)
        };
        Ok(Self {
            file,
            regular,
            operation,
            wake,
            _registration: registration,
        })
    }
    fn wait(&self) -> io::Result<()> {
        loop {
            melba_cdm::operation::check()?;
            if let Some(operation) = &self.operation {
                operation.check()?;
            }
            if self.regular {
                return Ok(());
            }
            let mut fds = [
                libc::pollfd {
                    fd: self.file.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // Both fds remain owned by this reader throughout poll. Cancellation
            // shuts the peer of wake down, waking every waiter without a timer.
            let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if let Some(operation) = &self.operation {
                operation.check()?;
            }
            if fds[0].revents != 0 {
                return Ok(());
            }
        }
    }
}
impl Read for SourceFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            self.wait()?;
            match self.file.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                result => return result,
            }
        }
    }
}
impl Seek for SourceFile {
    fn seek(&mut self, to: io::SeekFrom) -> io::Result<u64> {
        if let Some(operation) = &self.operation {
            operation.check()?;
        }
        self.file.seek(to)
    }
}
impl MediaSource for SourceFile {
    fn is_seekable(&self) -> bool {
        self.regular
    }
    fn byte_len(&self) -> Option<u64> {
        self.file.metadata().ok().map(|m| m.len())
    }
}
