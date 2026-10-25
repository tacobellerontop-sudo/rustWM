//! Tab bars over stacked tiles: one popup per stack, listing the stack's
//! windows. Clicking a tab shows that window.
//!
//! Like the focus border, the bars live on the event thread (which pumps
//! their messages). The core thread stores the wanted bars and posts a
//! message to a hidden host window; the event thread then creates, moves,
//! repaints or destroys bar windows to match.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use rwm_core::Event;
use rwm_core::platform::Stackbar;
use tracing::warn;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, FW_NORMAL, FillRect, InvalidateRect, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, HWND_MESSAGE, MA_NOACTIVATE, PostMessageW, RegisterClassW, SWP_NOACTIVATE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, WM_APP, WM_ERASEBKGND,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{PCWSTR, w};

/// Posted to the host window when [`WANTED`] changed.
const WM_UPDATE: u32 = WM_APP + 2;

const CLASS: PCWSTR = w!("rustwm-stackbar");

/// The hidden message-only window that receives [`WM_UPDATE`].
static HOST: AtomicIsize = AtomicIsize::new(0);
static WANTED: Mutex<Vec<Stackbar>> = Mutex::new(Vec::new());

thread_local! {
    /// The bar windows, in the order of [`WANTED`]. Event thread only.
    static BARS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

fn create_window(parent: Option<HWND>) -> Option<HWND> {
    // SAFETY: plain window creation with a class registered in `create`.
    let result = unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!("rustWM stackbar"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            parent,
            None,
            Some(instance.into()),
            None,
        )
    };
    match result {
        Ok(h) => Some(h),
        Err(e) => {
            warn!(error = %e, "could not create a stackbar window");
            None
        }
    }
}

/// Registers the window class and creates the host. Call on the event thread.
pub(crate) fn create() {
    // SAFETY: standard class registration with static wide strings.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
    }
    if let Some(host) = create_window(Some(HWND_MESSAGE)) {
        HOST.store(host.0 as isize, Ordering::Release);
        // The core may have asked for bars before the host existed.
        sync();
    }
}

/// Asks for exactly these bars to be drawn. Any thread.
pub(crate) fn update(bars: &[Stackbar]) {
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = bars.to_vec();
    let raw = HOST.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: posting to our own window never blocks.
        let _ = unsafe { PostMessageW(Some(HWND(raw as *mut _)), WM_UPDATE, WPARAM(0), LPARAM(0)) };
    }
}

fn wanted(index: usize) -> Option<Stackbar> {
    WANTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(index)
        .cloned()
}

/// Makes the bar windows match [`WANTED`]. Runs on the event thread.
fn sync() {
    let want = WANTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    BARS.with_borrow_mut(|bars| {
        while bars.len() > want.len() {
            if let Some(h) = bars.pop() {
                // SAFETY: our own window, destroyed on the thread that made it.
                let _ = unsafe { DestroyWindow(h) };
            }
        }
        while bars.len() < want.len() {
            match create_window(None) {
                Some(h) => bars.push(h),
                None => return,
            }
        }
        for (i, (h, bar)) in bars.iter().zip(&want).enumerate() {
            let r = bar.rect;
            // SAFETY: our own windows. The index tells the window which bar
            // it paints.
            unsafe {
                SetWindowLongPtrW(*h, GWLP_USERDATA, i as isize);
                let _ = SetWindowPos(
                    *h,
                    None,
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
                let _ = InvalidateRect(Some(*h), None, false);
            }
        }
    });
}

/// 0xRRGGBB to a COLORREF (0x00BBGGRR).
fn colorref(c: u32) -> COLORREF {
    COLORREF(((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff))
}

/// Halfway between two 0xRRGGBB colours.
fn mix(a: u32, b: u32) -> u32 {
    let ch = |shift: u32| ((((a >> shift) & 0xff) + ((b >> shift) & 0xff)) / 2) << shift;
    ch(16) | ch(8) | ch(0)
}

/// The tab `x` falls in, for a bar `width` wide with `count` tabs.
fn tab_at(x: i32, width: i32, count: usize) -> Option<usize> {
    if count == 0 || width <= 0 || x < 0 || x >= width {
        return None;
    }
    Some(((x as i64 * count as i64) / width as i64) as usize)
}

fn fill(dc: windows::Win32::Graphics::Gdi::HDC, r: &RECT, color: u32) {
    // SAFETY: the brush is deleted after use.
    unsafe {
        let brush = CreateSolidBrush(colorref(color));
        FillRect(dc, r, brush);
        let _ = DeleteObject(brush.into());
    }
}

fn paint(hwnd: HWND) {
    // SAFETY: our own window; reading back the index set in `sync`.
    let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as usize;
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: paired BeginPaint/EndPaint on our own window; every GDI object
    // created here is deselected and deleted.
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        if let Some(bar) = wanted(index).filter(|b| !b.tabs.is_empty()) {
            // Dark lines between the tabs.
            fill(dc, &client, mix(bar.inactive_color, 0));
            let height = client.bottom - client.top;
            let font = CreateFontW(
                -(height * 9 / 20).max(8),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let old = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            let n = bar.tabs.len() as i32;
            let width = client.right - client.left;
            for (i, (_, title)) in bar.tabs.iter().enumerate() {
                let i = i as i32;
                let mut r = RECT {
                    left: width * i / n,
                    top: 0,
                    right: width * (i + 1) / n - 1,
                    bottom: height,
                };
                let active = i as usize == bar.active;
                let (bg, fg) = match (active, bar.focused) {
                    (true, true) => (bar.active_color, bar.inactive_color),
                    (true, false) => (mix(bar.active_color, bar.inactive_color), bar.text_color),
                    (false, _) => (bar.inactive_color, bar.text_color),
                };
                fill(dc, &r, bg);
                SetTextColor(dc, colorref(fg));
                r.left += 6;
                r.right -= 6;
                let mut text: Vec<u16> = title.encode_utf16().collect();
                DrawTextW(
                    dc,
                    &mut text,
                    &mut r,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
            SelectObject(dc, old);
            let _ = DeleteObject(font.into());
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn clicked(hwnd: HWND, lparam: LPARAM) {
    // SAFETY: our own window.
    let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as usize;
    let Some(bar) = wanted(index) else {
        return;
    };
    let x = (lparam.0 & 0xffff) as i16 as i32;
    if let Some(i) = tab_at(x, bar.rect.w, bar.tabs.len()) {
        let (h, _) = bar.tabs[i];
        crate::events::send(Event::Command(format!("focus-window {}", h.0)));
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
            sync();
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            return LRESULT(0);
        }
        // Clicking a tab focuses its window, not the bar.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            clicked(hwnd, lparam);
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: forwarding the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_split_the_width_evenly() {
        assert_eq!(tab_at(0, 300, 3), Some(0));
        assert_eq!(tab_at(99, 300, 3), Some(0));
        assert_eq!(tab_at(100, 300, 3), Some(1));
        assert_eq!(tab_at(299, 300, 3), Some(2));
        assert_eq!(tab_at(300, 300, 3), None);
        assert_eq!(tab_at(5, 300, 0), None);
    }

    #[test]
    fn colours() {
        assert_eq!(colorref(0x112233), COLORREF(0x332211));
        assert_eq!(mix(0x000000, 0xffffff), 0x7f7f7f);
    }
}
