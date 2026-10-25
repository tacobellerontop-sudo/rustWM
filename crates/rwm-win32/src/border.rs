//! The coloured frame around the focused window: a click-through, topmost
//! popup shaped like a picture frame.
//!
//! The window lives on the event thread (which pumps its messages). The core
//! thread stores the wanted border and posts a message; the event thread
//! then moves, reshapes and repaints the frame.

use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use rwm_core::Border;
use tracing::{debug, warn};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateRectRgn, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
    InvalidateRect, PAINTSTRUCT, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA,
    PostMessageW, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, WM_APP, WM_ERASEBKGND, WM_NCHITTEST,
    WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

/// Posted to the border window when [`WANTED`] changed.
const WM_UPDATE: u32 = WM_APP + 1;

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static WANTED: Mutex<Option<Border>> = Mutex::new(None);

/// Creates the border window. Call on the event thread.
pub(crate) fn create() {
    // SAFETY: standard class registration and window creation with static
    // wide strings.
    let hwnd = unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("rustwm-border"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            w!("rustwm-border"),
            w!("rustWM border"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    };
    match hwnd {
        Ok(h) => {
            // SAFETY: our own fresh window. Layered windows are invisible
            // until given attributes; fully opaque here.
            let _ = unsafe { SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA) };
            HWND_RAW.store(h.0 as isize, Ordering::Release);
            // The core may have asked for a border before the window existed.
            apply(h);
        }
        Err(e) => warn!(error = %e, "could not create the focus border window"),
    }
}

/// Asks for the border to be drawn as `border`, or hidden. Any thread.
pub(crate) fn update(border: Option<Border>) {
    debug!(?border, "focus border");
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = border;
    let raw = HWND_RAW.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: posting to our own window never blocks.
        let _ = unsafe { PostMessageW(Some(HWND(raw as *mut _)), WM_UPDATE, WPARAM(0), LPARAM(0)) };
    }
}

/// Window `h` is sliding and its visible frame is at `frame` right now: if
/// the border is around it, move the border along.
pub(crate) fn track(h: rwm_core::WindowHandle, frame: rwm_core::Rect) {
    {
        let mut wanted = WANTED.lock().unwrap_or_else(|e| e.into_inner());
        match wanted.as_mut() {
            Some(b) if b.window == h && b.rect != frame => b.rect = frame,
            _ => return,
        }
    }
    let raw = HWND_RAW.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: posting to our own window never blocks.
        let _ = unsafe { PostMessageW(Some(HWND(raw as *mut _)), WM_UPDATE, WPARAM(0), LPARAM(0)) };
    }
}

fn wanted() -> Option<Border> {
    *WANTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Moves and reshapes the frame. Runs on the event thread.
fn apply(hwnd: HWND) {
    let Some(b) = wanted() else {
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
        return;
    };
    let (w, h, t) = (
        b.rect.w,
        b.rect.h,
        b.width.min(b.rect.w / 2).min(b.rect.h / 2),
    );
    // SAFETY: regions are created here and either handed to the system by
    // SetWindowRgn or deleted.
    unsafe {
        let frame = CreateRectRgn(0, 0, w, h);
        let hole = CreateRectRgn(t, t, w - t, h - t);
        CombineRgn(Some(frame), Some(frame), Some(hole), RGN_DIFF);
        let _ = DeleteObject(hole.into());
        SetWindowRgn(hwnd, Some(frame), false);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            b.rect.x,
            b.rect.y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_UPDATE => {
            apply(hwnd);
            return LRESULT(0);
        }
        WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let color = wanted().map_or(0, |b| b.color);
            // COLORREF is 0x00BBGGRR.
            let bgr = ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff);
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: paired BeginPaint/EndPaint on our own window; the brush
            // is deleted after use.
            unsafe {
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect = Default::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF(bgr));
                FillRect(dc, &rect, brush);
                let _ = DeleteObject(brush.into());
                let _ = EndPaint(hwnd, &ps);
            }
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: forwarding the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
