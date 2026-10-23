//! A file of the windows rustWM has hidden, so they can be shown again after a
//! crash, a kill, or from `rwm restore` without the daemon running.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rwm_core::WindowHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SW_SHOWNA, ShowWindowAsync};

fn path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("rustwm").join("hidden-windows.txt")
}

/// Rewrites the file with the current set. Errors are logged, never fatal:
/// losing the file only costs crash recovery, not correctness.
pub(crate) fn save(handles: &BTreeSet<WindowHandle>) {
    let path = path();
    let result = (|| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body: String = handles.iter().map(|h| format!("{}\n", h.0)).collect();
        std::fs::write(&path, body)
    })();
    if let Err(e) = result {
        tracing::warn!(path = %path.display(), error = %e, "failed to save hidden-window list");
    }
}

fn load() -> Vec<WindowHandle> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .map(WindowHandle)
        .collect()
}

/// Shows every window listed in the hidden-window file and clears it.
/// Returns how many windows were shown. Safe to call at any time.
pub fn restore_hidden_windows() -> usize {
    let mut shown = 0;
    for h in load() {
        let hwnd = HWND(h.0 as _);
        // SAFETY: both calls accept any handle value, including stale ones.
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_SHOWNA);
                shown += 1;
            }
        }
    }
    save(&BTreeSet::new());
    shown
}
