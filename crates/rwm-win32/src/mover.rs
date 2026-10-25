//! Moves windows, sliding them to their new place when animations are on.
//!
//! Every position change goes through this one thread, so an animation
//! still running can never overwrite a later instant move (hiding a window
//! off-screen, say). Rects here are outer window rects, as `SetWindowPos`
//! takes them.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rwm_core::{Rect, WindowHandle};
use tracing::debug;
use windows::Win32::UI::WindowsAndMessaging::{
    SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos,
};

use crate::window;

/// About 60 frames a second.
const FRAME: Duration = Duration::from_millis(16);

enum Job {
    /// Slide from `from` to `to` over `duration` (at once if zero). A window
    /// already sliding continues from where it is.
    Move {
        h: WindowHandle,
        from: Rect,
        to: Rect,
        duration: Duration,
        insets: Insets,
    },
    /// Answers once every earlier job is done and no animation is running.
    Flush(Sender<()>),
}

/// How far the visible frame is inside the outer rect (the invisible DWM
/// borders): left, top, right, bottom.
pub(crate) type Insets = (i32, i32, i32, i32);

struct Slide {
    from: Rect,
    to: Rect,
    start: Instant,
    duration: Duration,
    insets: Insets,
}

impl Slide {
    /// Where the window is at `now`, and whether the slide is over.
    fn at(&self, now: Instant) -> (Rect, bool) {
        let t = (now.saturating_duration_since(self.start).as_secs_f32()
            / self.duration.as_secs_f32().max(f32::EPSILON))
        .min(1.0);
        if t >= 1.0 {
            return (self.to, true);
        }
        // Ease out (cubic): fast start, gentle stop.
        let e = 1.0 - (1.0 - t).powi(3);
        let lerp = |a: i32, b: i32| a + ((b - a) as f32 * e).round() as i32;
        let (a, b) = (self.from, self.to);
        (
            Rect::new(
                lerp(a.x, b.x),
                lerp(a.y, b.y),
                lerp(a.w, b.w),
                lerp(a.h, b.h),
            ),
            false,
        )
    }
}

fn place(h: WindowHandle, r: Rect) {
    let hwnd = window::hwnd(h);
    // SAFETY: SetWindowPos accepts any handle and fails harmlessly on a stale
    // one; asynchronous, so a hung app cannot block this thread.
    let result = unsafe {
        SetWindowPos(
            hwnd,
            None,
            r.x,
            r.y,
            r.w,
            r.h,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        )
    };
    if let Err(e) = result {
        debug!(handle = ?h, error = %e, "SetWindowPos failed");
    }
}

fn run(jobs: Receiver<Job>) {
    let mut slides: HashMap<WindowHandle, Slide> = HashMap::new();
    let mut waiting: Vec<Sender<()>> = Vec::new();
    loop {
        let first = if slides.is_empty() {
            match jobs.recv() {
                Ok(j) => Some(j),
                Err(_) => return,
            }
        } else {
            match jobs.recv_timeout(FRAME) {
                Ok(j) => Some(j),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let now = Instant::now();
        for job in first.into_iter().chain(jobs.try_iter()) {
            match job {
                Job::Move {
                    h,
                    from,
                    to,
                    duration,
                    insets,
                } => {
                    let from = slides.remove(&h).map_or(from, |s| s.at(now).0);
                    if duration.is_zero() || from == to {
                        place(h, to);
                    } else {
                        slides.insert(
                            h,
                            Slide {
                                from,
                                to,
                                start: now,
                                duration,
                                insets,
                            },
                        );
                    }
                }
                Job::Flush(done) => waiting.push(done),
            }
        }
        slides.retain(|h, s| {
            let (r, done) = s.at(now);
            place(*h, r);
            let (l, t, rr, b) = s.insets;
            crate::border::track(*h, Rect::new(r.x + l, r.y + t, r.w - l - rr, r.h - t - b));
            !done
        });
        if slides.is_empty() {
            for done in waiting.drain(..) {
                let _ = done.send(());
            }
        }
    }
}

fn worker() -> &'static Mutex<Sender<Job>> {
    static WORKER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel();
        if let Err(e) = std::thread::Builder::new()
            .name("mover".into())
            .spawn(move || run(rx))
        {
            debug!(error = %e, "could not start the mover thread");
        }
        Mutex::new(tx)
    })
}

fn send(job: Job) {
    if let Ok(tx) = worker().lock() {
        let _ = tx.send(job);
    }
}

/// Slides window `h` from outer rect `from` to `to`; the focus border, if
/// it is around `h`, comes along.
pub(crate) fn slide(h: WindowHandle, from: Rect, to: Rect, duration: Duration, insets: Insets) {
    send(Job::Move {
        h,
        from,
        to,
        duration,
        insets,
    });
}

/// Puts window `h` at outer rect `to` at once, stopping any slide.
pub(crate) fn place_now(h: WindowHandle, to: Rect) {
    slide(h, to, to, Duration::ZERO, (0, 0, 0, 0));
}

/// Waits (at most a second) for every move to finish.
pub fn flush() {
    let (done, wait) = channel();
    send(Job::Flush(done));
    let _ = wait.recv_timeout(Duration::from_secs(1));
}
