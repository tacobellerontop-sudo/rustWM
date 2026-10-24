//! The named-pipe server. It only does pipe mechanics; what a request means
//! is up to the daemon's connection handler.

use std::fs::File;
use std::io;
use std::os::windows::io::FromRawHandle;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::HSTRING;

use crate::PIPE_NAME;

const BUFFER: u32 = 64 * 1024;

/// Another rustWM already owns the pipe.
#[derive(Debug)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rustWM is already running")
    }
}

impl std::error::Error for AlreadyRunning {}

/// Owns the pipe name for the life of the daemon.
pub struct Server {
    /// The first instance, created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so a
    /// second daemon fails to start instead of sharing the name.
    first: Option<HANDLE>,
}

// SAFETY: a pipe HANDLE may be used from any thread.
unsafe impl Send for Server {}

fn create(first: bool) -> io::Result<HANDLE> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: valid name and flags. The default security descriptor gives
    // write access only to the creating user, SYSTEM and administrators, and
    // remote clients are rejected.
    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(PIPE_NAME),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            None,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

impl Server {
    /// Claims the pipe name. Fails with [`AlreadyRunning`] (wrapped in an
    /// `io::Error`) if another daemon has it.
    pub fn bind() -> io::Result<Self> {
        match create(true) {
            Ok(h) => Ok(Self { first: Some(h) }),
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32) => {
                Err(io::Error::new(io::ErrorKind::AddrInUse, AlreadyRunning))
            }
            Err(e) => Err(e),
        }
    }

    /// Accepts clients forever, calling `handler` on its own thread for each.
    pub fn serve<F>(mut self, handler: F) -> io::Result<()>
    where
        F: Fn(File) + Send + Sync + Clone + 'static,
    {
        loop {
            let handle = match self.first.take() {
                Some(h) => h,
                None => create(false)?,
            };
            // SAFETY: `handle` is a fresh pipe instance; no OVERLAPPED.
            let connected = match unsafe { ConnectNamedPipe(handle, None) } {
                Ok(()) => true,
                // The client connected between Create and Connect: also fine.
                Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
            };
            if !connected {
                // SAFETY: closing the handle we created.
                let _ = unsafe { CloseHandle(handle) };
                continue;
            }
            // SAFETY: we own `handle`; the File closes it when dropped.
            let file = unsafe { File::from_raw_handle(handle.0) };
            let handler = handler.clone();
            std::thread::Builder::new()
                .name("ipc-client".into())
                .spawn(move || handler(file))?;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(h) = self.first.take() {
            // SAFETY: closing the handle we created.
            let _ = unsafe { CloseHandle(h) };
        }
    }
}
