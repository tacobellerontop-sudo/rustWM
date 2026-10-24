//! A file of the windows rustWM has hidden, so they can be shown again after a
//! crash, a kill, or from `rwm restore` without the daemon running.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rwm_core::{Rect, WindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, SW_SHOWNA, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    ShowWindowAsync,
};

/// Windows rustWM took off the screen: hidden (`None`), or moved off-screen
/// from the outer rect they had (`Some`).
pub(crate) type Hidden = BTreeMap<WindowHandle, Option<Rect>>;

fn path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("rustwm").join("hidden-windows.txt")
}

/// Rewrites the file with the current set. Errors are logged, never fatal:
/// losing the file only costs crash recovery, not correctness.
pub(crate) fn save(handles: &Hidden) {
    let path = path();
    let result = (|| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body: String = handles
            .iter()
            .map(|(h, rect)| match rect {
                None => format!("{}\n", h.0),
                Some(r) => format!("{} {} {} {} {}\n", h.0, r.x, r.y, r.w, r.h),
            })
            .collect();
        std::fs::write(&path, body)
    })();
    if let Err(e) = result {
        tracing::warn!(path = %path.display(), error = %e, "failed to save hidden-window list");
    }
}

/// One line: a handle, optionally followed by the rect to move it back to.
fn parse_line(line: &str) -> Option<(WindowHandle, Option<Rect>)> {
    let nums: Vec<i64> = line
        .split_whitespace()
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let h = WindowHandle(*nums.first()? as isize);
    let rect = match nums[1..] {
        [x, y, w, h] => Some(Rect::new(x as i32, y as i32, w as i32, h as i32)),
        _ => None,
    };
    Some((h, rect))
}

fn load() -> Vec<(WindowHandle, Option<Rect>)> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(parse_line)
        .collect()
}

/// Shows every window listed in the hidden-window file and clears it.
/// Returns how many windows were shown. Safe to call at any time.
pub fn restore_hidden_windows() -> usize {
    let mut shown = 0;
    for (h, rect) in load() {
        let hwnd = HWND(h.0 as _);
        // SAFETY: these calls accept any handle value, including stale ones,
        // and are asynchronous.
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() {
                if let Some(r) = rect {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.x,
                        r.y,
                        r.w,
                        r.h,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    );
                }
                let _ = ShowWindowAsync(hwnd, SW_SHOWNA);
                shown += 1;
            }
        }
    }
    save(&Hidden::new());
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_with_and_without_a_rect() {
        assert_eq!(parse_line("42"), Some((WindowHandle(42), None)));
        assert_eq!(
            parse_line("42 -10 20 800 600"),
            Some((WindowHandle(42), Some(Rect::new(-10, 20, 800, 600))))
        );
        assert_eq!(parse_line("x"), None);
    }
}
