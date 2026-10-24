use std::fs::{File, OpenOptions};
use std::io::{self, BufReader};
use std::time::{Duration, Instant};

use crate::{EventMessage, PIPE_NAME, Request, Response, read_message, write_message};

/// `ERROR_PIPE_BUSY`: every server instance is serving another client.
const ERROR_PIPE_BUSY: i32 = 231;

/// A connection to the running daemon.
pub struct Client {
    reader: BufReader<File>,
    writer: File,
}

impl Client {
    /// Connects to the daemon, retrying briefly while the pipe is busy.
    /// Fails with `NotFound` when rustWM is not running.
    pub fn connect() -> io::Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match OpenOptions::new().read(true).write(true).open(PIPE_NAME) {
                Ok(file) => {
                    return Ok(Self {
                        writer: file.try_clone()?,
                        reader: BufReader::new(file),
                    });
                }
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Sends one request and waits for its response.
    pub fn request(&mut self, request: &Request) -> io::Result<Response> {
        write_message(&mut self.writer, request)?;
        read_message(&mut self.reader)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "rustWM closed the connection")
        })
    }

    /// Runs a command string, like a key binding would.
    pub fn command(&mut self, cmd: &str) -> io::Result<Response> {
        self.request(&Request::Command { cmd: cmd.into() })
    }

    /// Subscribes to `topics` and returns the stream of events.
    pub fn subscribe(
        mut self,
        topics: Vec<String>,
    ) -> io::Result<impl Iterator<Item = io::Result<EventMessage>>> {
        let response = self.request(&Request::Subscribe { subscribe: topics })?;
        if !response.ok {
            return Err(io::Error::other(response.error.unwrap_or_default()));
        }
        let mut reader = self.reader;
        Ok(std::iter::from_fn(move || {
            read_message(&mut reader).transpose()
        }))
    }
}
