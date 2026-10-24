//! Starting the hotkey daemon (kerys by default) alongside rustWM.
//!
//! rustWM has no keyboard handling of its own. The daemon watches the keys
//! and sends commands over the pipe, like `rwm` does. It is started in a job
//! object that kills it when the job handle closes, so it goes away with
//! rustWM however rustWM exits, crashes included, and never keeps swallowing
//! Win+key chords with no window manager behind them.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tracing::info;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

/// No console window for the background daemon.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The running hotkey daemon. Dropping it stops the daemon.
pub struct HotkeyDaemon {
    job: HANDLE,
}

// SAFETY: a job HANDLE may be used and closed from any thread.
unsafe impl Send for HotkeyDaemon {}

/// A bare program name is looked for next to rustwm.exe first, so the exes
/// from one zip find each other without being on PATH.
fn resolve(program: &str) -> PathBuf {
    let bare = !program.contains(['\\', '/']);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(program)))
        .filter(|p| bare && p.is_file());
    beside.unwrap_or_else(|| Path::new(program).to_path_buf())
}

fn kill_on_close_job() -> io::Result<HANDLE> {
    // SAFETY: an anonymous job with default security.
    let job = unsafe { CreateJobObjectW(None, None) }?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is the struct this information class expects, and it
    // outlives the call.
    let set = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if let Err(e) = set {
        // SAFETY: closing the job we just created.
        let _ = unsafe { CloseHandle(job) };
        return Err(e.into());
    }
    Ok(job)
}

impl HotkeyDaemon {
    /// Starts `command_line` (a program plus arguments) in the background.
    pub fn start(command_line: &str) -> io::Result<Self> {
        let (program, args) = rwm_core::cmdline::split_program(command_line)
            .ok_or_else(|| io::Error::other("empty or malformed command line"))?;
        let path = resolve(program);
        let mut cmd = Command::new(&path);
        if !args.is_empty() {
            cmd.raw_arg(args);
        }
        let job = kill_on_close_job()?;
        let child = cmd
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())));
        let child = match child {
            Ok(c) => c,
            Err(e) => {
                // SAFETY: closing the job we created.
                let _ = unsafe { CloseHandle(job) };
                return Err(e);
            }
        };
        // SAFETY: both handles are valid; the child's stays open while
        // `child` lives.
        let assigned = unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) };
        if let Err(e) = assigned {
            tracing::warn!(error = %e, "hotkey daemon will not stop with rustWM");
        }
        info!(program = %path.display(), pid = child.id(), "started hotkey daemon");
        Ok(Self { job })
    }
}

impl Drop for HotkeyDaemon {
    fn drop(&mut self) {
        // SAFETY: closing the job we own; this kills the daemon.
        let _ = unsafe { CloseHandle(self.job) };
    }
}

/// Whether a kerys is running: it keeps a message-only window of class
/// `kerys` for `kerys stop`, whoever started it.
pub fn kerys_running() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, HWND_MESSAGE};
    use windows::core::w;
    // SAFETY: looking up a window by its static class name.
    unsafe { FindWindowExW(Some(HWND_MESSAGE), None, w!("kerys"), None) }
        .is_ok_and(|h| !h.is_invalid())
}
