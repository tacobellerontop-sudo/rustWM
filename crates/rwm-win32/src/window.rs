//! Reading window properties and deciding which windows rustWM manages.

use rwm_core::{Rect, WindowHandle, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, WS_CAPTION, WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_THICKFRAME,
};
use windows::core::{BOOL, PWSTR};

use crate::monitor::rect_from;

/// Shell and system classes that are visible top-level windows but must never
/// be tiled.
const IGNORED_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "NotifyIconOverflowWindow",
    "MultitaskingViewFrame",
    "ForegroundStaging",
];

pub(crate) fn hwnd(h: WindowHandle) -> HWND {
    HWND(h.0 as _)
}

pub(crate) fn handle(hwnd: HWND) -> WindowHandle {
    WindowHandle(hwnd.0 as isize)
}

pub(crate) fn exists(h: HWND) -> bool {
    // SAFETY: IsWindow accepts any handle value.
    unsafe { IsWindow(Some(h)).as_bool() }
}

fn title(h: HWND) -> String {
    // SAFETY: the buffer is sized from GetWindowTextLengthW plus the terminator.
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn class(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is a valid writable slice.
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn process_id(h: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer.
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// The executable file name of the process owning `h`, e.g. `firefox.exe`.
fn exe(pid: u32) -> String {
    // SAFETY: the process handle is closed before returning; the buffer and
    // its length are passed together.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        if ok.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or_default().to_string()
    }
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: `cloaked` is a u32, the documented type for DWMWA_CLOAKED.
    let r = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            size_of::<u32>() as u32,
        )
    };
    r.is_ok() && cloaked != 0
}

/// The full window rect including invisible resize borders.
pub(crate) fn window_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    // SAFETY: `r` is a valid out-pointer.
    unsafe { GetWindowRect(h, &mut r) }.ok().map(|_| r)
}

/// The visible frame, without the invisible DWM borders Windows 10/11 add.
pub(crate) fn frame_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    // SAFETY: `r` is a RECT, the documented type for this attribute.
    let ok = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
    };
    match ok {
        Ok(()) => Some(r),
        Err(_) => window_rect(h),
    }
}

/// Converts a desired visible frame into the `SetWindowPos` rect, by adding
/// back the invisible border the window currently has on each side.
pub(crate) fn outer_rect_for(h: HWND, frame: Rect) -> Rect {
    match (window_rect(h), frame_rect(h)) {
        (Some(outer), Some(visible)) => {
            let left = visible.left - outer.left;
            let top = visible.top - outer.top;
            let right = outer.right - visible.right;
            let bottom = outer.bottom - visible.bottom;
            Rect::new(
                frame.x - left,
                frame.y - top,
                frame.w + left + right,
                frame.h + top + bottom,
            )
        }
        _ => frame,
    }
}

fn is_manageable(h: HWND, class: &str, title: &str, pid: u32, rect: Rect) -> bool {
    // SAFETY: all of these accept any window handle.
    let (visible, style, ex_style, owner) = unsafe {
        (
            IsWindowVisible(h).as_bool(),
            GetWindowLongPtrW(h, GWL_STYLE) as u32,
            GetWindowLongPtrW(h, GWL_EXSTYLE) as u32,
            GetWindow(h, GW_OWNER).ok(),
        )
    };
    // SAFETY: no preconditions.
    let own_pid = unsafe { GetCurrentProcessId() };
    visible
        && style & WS_CHILD.0 == 0
        && (style & WS_CAPTION.0 == WS_CAPTION.0 || style & WS_THICKFRAME.0 != 0)
        && ex_style & WS_EX_TOOLWINDOW.0 == 0
        && ex_style & WS_EX_NOACTIVATE.0 == 0
        && owner.is_none_or(|o| o.is_invalid())
        && !title.is_empty()
        && !rect.is_empty()
        && pid != own_pid
        && !IGNORED_CLASSES.contains(&class)
        && !is_cloaked(h)
}

pub(crate) fn info(h: HWND) -> Option<WindowInfo> {
    if !exists(h) {
        return None;
    }
    let title = title(h);
    let class = class(h);
    let pid = process_id(h);
    let rect = frame_rect(h).map(rect_from).unwrap_or_default();
    let manageable = is_manageable(h, &class, &title, pid, rect);
    Some(WindowInfo {
        handle: handle(h),
        // Only pay for OpenProcess on windows we might actually manage.
        exe: if manageable { exe(pid) } else { String::new() },
        title,
        class,
        rect,
        // SAFETY: IsIconic accepts any handle.
        minimized: unsafe { IsIconic(h).as_bool() },
        manageable,
    })
}

unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HWND>` from `enumerate`, alive for the call.
    let out = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    out.push(h);
    true.into()
}

/// Every top-level window, topmost first.
pub(crate) fn enumerate() -> Vec<WindowInfo> {
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    handles.into_iter().filter_map(info).collect()
}
