use std::collections::BTreeSet;
use std::os::windows::process::CommandExt;

use rwm_core::platform::Result;
use rwm_core::{MonitorInfo, Platform, PlatformError, Rect, WindowHandle, WindowInfo};
use tracing::{info, warn};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_MOUSE, SendInput};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetShellWindow, IsIconic, IsZoomed, PostMessageW, SW_HIDE,
    SW_RESTORE, SW_SHOWNA, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    SetCursorPos, SetForegroundWindow, SetWindowPos, ShowWindowAsync, WM_CLOSE,
};

use crate::{hidden, monitor, window};

/// Gives a console app its own window; GUI apps ignore the flag.
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Makes all coordinates physical pixels on every monitor. Call once, before
/// any window or monitor is queried.
pub fn enable_dpi_awareness() {
    // SAFETY: no preconditions; fails harmlessly if already set (e.g. by a manifest).
    if let Err(e) =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
    {
        warn!(error = %e, "could not set per-monitor DPI awareness");
    }
}

/// The real platform, backed by the `windows` crate.
///
/// Every call that touches another process's window is asynchronous
/// (`ShowWindowAsync`, `SWP_ASYNCWINDOWPOS`) so a hung app can never block
/// the window manager.
#[derive(Debug, Default)]
pub struct Win32Platform {
    /// Windows we hid, mirrored to disk so they survive a crash.
    hidden: BTreeSet<WindowHandle>,
}

impl Win32Platform {
    /// Creates the platform, first showing any windows a previous run hid and
    /// never got to restore.
    pub fn new() -> Self {
        let recovered = hidden::restore_hidden_windows();
        if recovered > 0 {
            info!(
                count = recovered,
                "restored windows hidden by a previous run"
            );
        }
        Self::default()
    }

    fn checked(&self, h: WindowHandle) -> Result<windows::Win32::Foundation::HWND> {
        let hwnd = window::hwnd(h);
        if window::exists(hwnd) {
            Ok(hwnd)
        } else {
            Err(PlatformError::WindowGone(h))
        }
    }
}

impl Platform for Win32Platform {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        let monitors = monitor::enumerate();
        if monitors.is_empty() {
            return Err(PlatformError::Os("no monitors found".into()));
        }
        Ok(monitors)
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(window::enumerate())
    }

    fn window_info(&self, handle: WindowHandle) -> Option<WindowInfo> {
        window::info(window::hwnd(handle))
    }

    fn min_size(&self, handle: WindowHandle) -> Option<(i32, i32)> {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, MINMAXINFO, SM_CXMINTRACK, SM_CYMINTRACK, SMTO_ABORTIFHUNG,
            SMTO_ERRORONEXIT, SendMessageTimeoutW, WM_GETMINMAXINFO,
        };
        let hwnd = self.checked(handle).ok()?;
        // SAFETY: no preconditions.
        let system = unsafe {
            (
                GetSystemMetrics(SM_CXMINTRACK),
                GetSystemMetrics(SM_CYMINTRACK),
            )
        };
        // Windows fills in defaults before an app sees this message and the
        // app only changes what it cares about, so start from the defaults.
        let mut mmi = MINMAXINFO::default();
        mmi.ptMinTrackSize.x = system.0;
        mmi.ptMinTrackSize.y = system.1;
        mmi.ptMaxTrackSize.x = i32::MAX;
        mmi.ptMaxTrackSize.y = i32::MAX;
        // SAFETY: WM_GETMINMAXINFO is a system message, so Windows copies the
        // MINMAXINFO into the target process and back. The timeout keeps a
        // hung app from blocking us.
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETMINMAXINFO,
                WPARAM(0),
                LPARAM(&mut mmi as *mut MINMAXINFO as isize),
                SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
                100,
                None,
            )
        };
        if ok.0 == 0 {
            return None;
        }
        let (w, h) = (mmi.ptMinTrackSize.x, mmi.ptMinTrackSize.y);
        // The system default means the app sets no minimum of its own.
        if w <= system.0 && h <= system.1 {
            return None;
        }
        // The minimum is for the whole window; we lay out visible frames.
        let (bw, bh) = match (window::window_rect(hwnd), window::frame_rect(hwnd)) {
            (Some(o), Some(f)) => (
                (o.right - o.left) - (f.right - f.left),
                (o.bottom - o.top) - (f.bottom - f.top),
            ),
            _ => (0, 0),
        };
        tracing::debug!(?handle, w, h, "minimum window size");
        Some(((w - bw).max(0), (h - bh).max(0)))
    }

    fn foreground(&self) -> Option<WindowHandle> {
        // SAFETY: GetForegroundWindow has no preconditions.
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_invalid()).then(|| window::handle(hwnd))
    }

    fn set_rects(&mut self, moves: &[(WindowHandle, Rect)]) -> Result<()> {
        let mut first_err = None;
        for &(h, frame) in moves {
            let hwnd = match self.checked(h) {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    first_err.get_or_insert(e);
                    continue;
                }
            };
            // SAFETY: `hwnd` was just validated; both calls are asynchronous
            // for windows owned by other threads, so a hung app cannot block us.
            unsafe {
                if IsZoomed(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                }
                let r = window::outer_rect_for(hwnd, frame);
                if let Err(e) = SetWindowPos(
                    hwnd,
                    None,
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
                ) {
                    // Access denied almost always means an elevated window.
                    warn!(handle = ?h, error = %e, "SetWindowPos failed");
                    first_err.get_or_insert(PlatformError::AccessDenied(h));
                }
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    fn show(&mut self, handle: WindowHandle) -> Result<()> {
        if self.hidden.remove(&handle) {
            hidden::save(&self.hidden);
        }
        let hwnd = self.checked(handle)?;
        // SAFETY: validated handle; asynchronous call.
        let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWNA) };
        Ok(())
    }

    fn hide(&mut self, handle: WindowHandle) -> Result<()> {
        let hwnd = self.checked(handle)?;
        // Record before hiding, so a crash in between still restores it.
        self.hidden.insert(handle);
        hidden::save(&self.hidden);
        // SAFETY: validated handle; asynchronous call.
        let _ = unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
        Ok(())
    }

    fn focus(&mut self, handle: WindowHandle) -> Result<()> {
        let hwnd = self.checked(handle)?;
        // SAFETY: validated handle. Windows only lets the process that received
        // the last input event take the foreground, so we first inject an empty
        // mouse event to become that process (komorebi's trick; it avoids the
        // menu-bar side effects of a synthetic Alt tap).
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            }
            let input = INPUT {
                r#type: INPUT_MOUSE,
                ..Default::default()
            };
            SendInput(&[input], size_of::<INPUT>() as i32);
            if !SetForegroundWindow(hwnd).as_bool() {
                return Err(PlatformError::Os(format!(
                    "SetForegroundWindow refused for {handle:?}"
                )));
            }
        }
        Ok(())
    }

    fn focus_desktop(&mut self) -> Result<()> {
        // SAFETY: GetShellWindow takes no arguments; the input trick is the
        // same as in `focus`.
        unsafe {
            let desktop = GetShellWindow();
            if desktop.is_invalid() {
                return Err(PlatformError::Os("no shell window".into()));
            }
            let input = INPUT {
                r#type: INPUT_MOUSE,
                ..Default::default()
            };
            SendInput(&[input], size_of::<INPUT>() as i32);
            if !SetForegroundWindow(desktop).as_bool() {
                return Err(PlatformError::Os(
                    "SetForegroundWindow refused for the desktop".into(),
                ));
            }
        }
        Ok(())
    }

    fn close(&mut self, handle: WindowHandle) -> Result<()> {
        let hwnd = self.checked(handle)?;
        // SAFETY: validated handle; PostMessage never blocks on the target.
        unsafe { PostMessageW(Some(hwnd), WM_CLOSE, Default::default(), Default::default()) }
            .map_err(|e| PlatformError::Os(e.to_string()))
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let mut p = POINT::default();
        // SAFETY: `p` is a valid out-pointer.
        unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
    }

    fn set_border(&mut self, border: Option<rwm_core::Border>) {
        crate::border::update(border);
    }

    fn set_cursor_pos(&mut self, x: i32, y: i32) -> Result<()> {
        // SAFETY: plain coordinates; the process is per-monitor DPI aware, so
        // they are physical pixels like everything else.
        unsafe { SetCursorPos(x, y) }.map_err(|e| PlatformError::Os(e.to_string()))
    }

    fn set_opacity(&mut self, handle: WindowHandle, alpha: u8) -> Result<()> {
        self.checked(handle)?;
        crate::opacity::set(handle, alpha);
        Ok(())
    }

    fn spawn(&mut self, command_line: &str) -> Result<()> {
        let spawn_err = |reason: String| PlatformError::Spawn {
            cmd: command_line.into(),
            reason,
        };
        let (program, args) = rwm_core::cmdline::split_program(command_line)
            .ok_or_else(|| spawn_err("empty or malformed command line".into()))?;
        let mut cmd = std::process::Command::new(program);
        if !args.is_empty() {
            cmd.raw_arg(args);
        }
        cmd.creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(drop)
            .map_err(|e| spawn_err(e.to_string()))
    }
}
