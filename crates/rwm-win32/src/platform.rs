use rwm_core::platform::Result;
use rwm_core::{MonitorInfo, Platform, PlatformError, Rect, WindowHandle, WindowInfo};

/// The real platform, backed by the `windows` crate.
///
/// Window enumeration, hooks, positioning and DPI land in M1; until then every
/// call reports that it is not implemented.
#[derive(Debug, Default)]
pub struct Win32Platform {}

impl Win32Platform {
    pub fn new() -> Self {
        Self {}
    }
}

fn todo_m1<T>() -> Result<T> {
    Err(PlatformError::Os("not implemented yet (M1)".into()))
}

impl Platform for Win32Platform {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        todo_m1()
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        todo_m1()
    }

    fn window_info(&self, _handle: WindowHandle) -> Option<WindowInfo> {
        None
    }

    fn foreground(&self) -> Option<WindowHandle> {
        // SAFETY: GetForegroundWindow has no preconditions.
        let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        (!hwnd.is_invalid()).then_some(WindowHandle(hwnd.0 as isize))
    }

    fn set_rects(&mut self, _moves: &[(WindowHandle, Rect)]) -> Result<()> {
        todo_m1()
    }

    fn show(&mut self, _handle: WindowHandle) -> Result<()> {
        todo_m1()
    }

    fn hide(&mut self, _handle: WindowHandle) -> Result<()> {
        todo_m1()
    }

    fn focus(&mut self, _handle: WindowHandle) -> Result<()> {
        todo_m1()
    }

    fn spawn(&mut self, _command_line: &str) -> Result<()> {
        todo_m1()
    }
}
