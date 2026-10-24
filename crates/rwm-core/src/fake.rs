//! An in-memory [`Platform`] for tests.
//!
//! It keeps a list of windows and monitors, applies moves/hides/focus to that
//! list the way a real desktop would, and records every call so tests can
//! assert on exactly what the core asked the OS to do.

use std::collections::BTreeMap;

use crate::platform::Result;
use crate::{Border, MonitorInfo, Platform, PlatformError, Rect, WindowHandle, WindowInfo};

/// One call the core made into the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    SetRects(Vec<(WindowHandle, Rect)>),
    Show(WindowHandle),
    Hide(WindowHandle),
    Focus(WindowHandle),
    FocusDesktop,
    Close(WindowHandle),
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
    pub cursor: Option<(i32, i32)>,
    /// Minimum sizes some windows insist on; `set_rects` will not go below.
    pub min_size: BTreeMap<WindowHandle, (i32, i32)>,
    /// Command lines starting with any of these fail to spawn.
    pub failing_spawns: Vec<String>,
    /// The focus border as last drawn.
    pub border: Option<Border>,
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

    /// Two 1920x1080 monitors side by side; the left one is primary.
    pub fn dual_monitor() -> Self {
        let mut p = Self::single_monitor();
        p.monitors.push(MonitorInfo {
            id: r"\\.\DISPLAY2".into(),
            rect: Rect::new(1920, 0, 1920, 1080),
            work_area: Rect::new(1920, 0, 1920, 1080),
            scale: 1.0,
            primary: false,
        });
        p
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
                resizable: true,
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

    /// Places a window, as if the user dragged it there.
    pub fn place(&mut self, handle: WindowHandle, rect: Rect) {
        if let Some(w) = self.windows.get_mut(&handle) {
            w.rect = rect;
        }
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
            let (min_w, min_h) = self.min_size.get(handle).copied().unwrap_or((0, 0));
            let r = Rect::new(rect.x, rect.y, rect.w.max(min_w), rect.h.max(min_h));
            self.get_mut(*handle)?.rect = r;
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

    fn focus_desktop(&mut self) -> Result<()> {
        self.calls.push(Call::FocusDesktop);
        self.foreground = None;
        Ok(())
    }

    fn close(&mut self, handle: WindowHandle) -> Result<()> {
        self.calls.push(Call::Close(handle));
        self.get_mut(handle)?;
        Ok(())
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        self.cursor
    }

    fn spawn(&mut self, command_line: &str) -> Result<()> {
        self.calls.push(Call::Spawn(command_line.into()));
        if self
            .failing_spawns
            .iter()
            .any(|p| command_line.starts_with(p.as_str()))
        {
            return Err(PlatformError::Spawn {
                cmd: command_line.into(),
                reason: "not found".into(),
            });
        }
        Ok(())
    }

    fn set_border(&mut self, border: Option<Border>) {
        self.border = border;
    }
}
