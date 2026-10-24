//! Focus follows mouse: a small thread that watches which top-level window
//! the cursor rests on and reports it as [`Event::MouseOver`].

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rwm_core::{Event, WindowHandle};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, WindowFromPoint,
};

use crate::events::send;

/// How often the cursor is checked.
const POLL: Duration = Duration::from_millis(10);
/// How long the cursor must rest on a window before it gets focus. Just long
/// enough that flicking the mouse across a window doesn't focus it.
const SETTLE: Duration = Duration::from_millis(25);

static ENABLED: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();

/// Turns focus-follows-mouse on or off.
pub fn set_focus_follows_mouse(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    if on {
        START.call_once(|| {
            let _ = std::thread::Builder::new().name("hover".into()).spawn(run);
        });
    }
}

fn buttons_down() -> bool {
    // SAFETY: no preconditions; the high bit means "down now".
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
        .into_iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0)
}

fn window_under_cursor() -> Option<WindowHandle> {
    let mut p = POINT::default();
    // SAFETY: `p` is a valid out-pointer; the other calls only read handles.
    unsafe {
        GetCursorPos(&mut p).ok()?;
        let hwnd = WindowFromPoint(p);
        if hwnd.is_invalid() {
            return None;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        Some(WindowHandle(
            if root.is_invalid() { hwnd.0 } else { root.0 } as isize,
        ))
    }
}

fn run() {
    // The window under the cursor, since when, and whether it was reported.
    let mut current: Option<(WindowHandle, Instant, bool)> = None;
    loop {
        std::thread::sleep(POLL);
        if !ENABLED.load(Ordering::Relaxed) || buttons_down() {
            current = None;
            continue;
        }
        let Some(h) = window_under_cursor() else {
            current = None;
            continue;
        };
        match &mut current {
            Some((w, since, reported)) if *w == h => {
                if !*reported && since.elapsed() >= SETTLE {
                    *reported = true;
                    send(Event::MouseOver(h));
                }
            }
            _ => current = Some((h, Instant::now(), false)),
        }
    }
}
