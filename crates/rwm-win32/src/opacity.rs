//! Window translucency for `inactive_opacity`.
//!
//! Making a window layered changes its extended style, which sends it
//! messages and waits for the answer. A hung app would block the window
//! manager, so the work happens on a thread of its own.

use std::collections::BTreeSet;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rwm_core::WindowHandle;
use tracing::debug;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
    WS_EX_LAYERED,
};

use crate::window;

enum Job {
    Set(WindowHandle, u8),
    /// Answers once every earlier job is done.
    Flush(Sender<()>),
}

fn worker() -> &'static Mutex<Sender<Job>> {
    static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("opacity".into())
            .spawn(move || {
                // Windows we made layered, so we only undo our own change and
                // leave apps that are layered by design alone.
                let mut ours = BTreeSet::new();
                for job in rx {
                    match job {
                        Job::Set(h, alpha) => apply(&mut ours, h, alpha),
                        Job::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            debug!(error = %e, "could not start the opacity thread");
        }
        Mutex::new(tx)
    })
}

fn apply(ours: &mut BTreeSet<WindowHandle>, h: WindowHandle, alpha: u8) {
    let hwnd = window::hwnd(h);
    if !window::exists(hwnd) {
        ours.remove(&h);
        return;
    }
    let layered = WS_EX_LAYERED.0 as isize;
    // SAFETY: the handle was checked just above; these calls accept any
    // window and fail harmlessly if it went away in between.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if alpha == 255 {
            if ours.remove(&h) {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !layered);
            }
            return;
        }
        if ex & layered == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | layered);
            ours.insert(h);
        } else if !ours.contains(&h) {
            return;
        }
        if let Err(e) = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA) {
            debug!(handle = ?h, error = %e, "SetLayeredWindowAttributes failed");
        }
    }
}

/// Queues an opacity change; 255 undoes it.
pub fn set(h: WindowHandle, alpha: u8) {
    if let Ok(tx) = worker().lock() {
        let _ = tx.send(Job::Set(h, alpha));
    }
}

/// Waits (briefly) for queued changes, so windows are opaque again before
/// rustWM exits.
pub fn flush() {
    let (done, wait) = channel();
    if let Ok(tx) = worker().lock() {
        if tx.send(Job::Flush(done)).is_err() {
            return;
        }
    }
    let _ = wait.recv_timeout(Duration::from_secs(1));
}
