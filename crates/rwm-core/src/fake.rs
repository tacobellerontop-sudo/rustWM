//! An in-memory [`Platform`] for tests.
//!
//! It keeps a list of windows and monitors, applies moves/hides/focus to that
//! list the way a real desktop would, and records every call so tests can
//! assert on exactly what the core asked the OS to do.

use std::collections::BTreeMap;

use crate::platform::Result;
use crate::{MonitorInfo, Platform, PlatformError, Rect, WindowHandle, WindowInfo};

/// One call the core made into the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    SetRects(Vec<(WindowHandle, Rect)>),
    Show(WindowHandle),
    Hide(WindowHandle),
    Focus(WindowHandle),
    Spawn(String),
}

#[derive(Debug, Clone, Default)]
pub struct FakePlatform {
    pub monitors: Vec<MonitorInfo>,
    /// Windows by handle. Hidden windows stay here with `manageable = false`
    /// so they can be shown again, like on a real desktop.
    pub windows: BTreeMap<WindowHandle, WindowInfo>,
    pub hidden: Vec<WindowHandle>,
    pub foreground: Option<WindowHandle>,
    pub calls: Vec<Call>,
}

impl FakePlatform {
    /// One 1920x1080 monitor with a 40px taskbar at the bottom.
    pub fn single_monitor() -> Self {
        Self {
            monitors: vec![MonitorInfo {
                id: r"\\.\DISPLAY1".into(),
                rect: Rect::new(0, 0, 1920, 1080),
                work_area: Rect::new(0, 0, 1920, 1040),
                scale: 1.0,
                primary: true,
            }],
            ..Self::default()
        }
    }

    /// Adds a normal, manageable app window and returns its handle.
    pub fn add_window(&mut self, id: isize, exe: &str, title: &str) -> WindowHandle {
        let handle = WindowHandle(id);
        self.windows.insert(
            handle,
            WindowInfo {
                handle,
                title: title.into(),
                class: format!("{exe}-class"),
                exe: exe.into(),
                rect: Rect::new(100, 100, 800, 600),
                minimized: false,
                manageable: true,
            },
        );
        handle
    }

    /// Removes a window, as if the app closed it.
    pub fn close_window(&mut self, handle: WindowHandle) {
        self.windows.remove(&handle);
        self.hidden.retain(|h| *h != handle);
        if self.foreground == Some(handle) {
            self.foreground = None;
        }
    }

    pub fn is_hidden(&self, handle: WindowHandle) -> bool {
        self.hidden.contains(&handle)
    }

    pub fn rect_of(&self, handle: WindowHandle) -> Option<Rect> {
        self.windows.get(&handle).map(|w| w.rect)
    }

    fn get_mut(&mut self, handle: WindowHandle) -> Result<&mut WindowInfo> {
        self.windows
            .get_mut(&handle)
            .ok_or(PlatformError::WindowGone(handle))
    }
}

impl Platform for FakePlatform {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        Ok(self.monitors.clone())
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(self.windows.values().cloned().collect())
    }

    fn window_info(&self, handle: WindowHandle) -> Option<WindowInfo> {
        self.windows.get(&handle).cloned()
    }

    fn foreground(&self) -> Option<WindowHandle> {
        self.foreground
    }

    fn set_rects(&mut self, moves: &[(WindowHandle, Rect)]) -> Result<()> {
        self.calls.push(Call::SetRects(moves.to_vec()));
        for (handle, rect) in moves {
            self.get_mut(*handle)?.rect = *rect;
        }
        Ok(())
    }

    fn show(&mut self, handle: WindowHandle) -> Result<()> {
        self.calls.push(Call::Show(handle));
        self.get_mut(handle)?.manageable = true;
        self.hidden.retain(|h| *h != handle);
        Ok(())
    }

    fn hide(&mut self, handle: WindowHandle) -> Result<()> {
        self.calls.push(Call::Hide(handle));
        self.get_mut(handle)?.manageable = false;
        if !self.hidden.contains(&handle) {
            self.hidden.push(handle);
        }
        Ok(())
    }

    fn focus(&mut self, handle: WindowHandle) -> Result<()> {
        self.calls.push(Call::Focus(handle));
        self.get_mut(handle)?;
        self.foreground = Some(handle);
        Ok(())
    }

    fn spawn(&mut self, command_line: &str) -> Result<()> {
        self.calls.push(Call::Spawn(command_line.into()));
        Ok(())
    }
}
