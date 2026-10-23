use serde::{Deserialize, Serialize};

use crate::{Rect, WindowHandle, WindowInfo};

/// A physical display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Stable-ish identifier (the device name on Windows, e.g. `\\.\DISPLAY1`).
    pub id: String,
    /// Full monitor bounds.
    pub rect: Rect,
    /// Bounds minus the taskbar and other app bars.
    pub work_area: Rect,
    /// Scale factor, 1.0 = 96 DPI.
    pub scale: f32,
    pub primary: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("window {0:?} no longer exists")]
    WindowGone(WindowHandle),
    #[error("access denied to window {0:?} (elevated?)")]
    AccessDenied(WindowHandle),
    #[error("failed to spawn `{cmd}`: {reason}")]
    Spawn { cmd: String, reason: String },
    #[error("os error: {0}")]
    Os(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// Everything the window manager needs from the operating system.
///
/// The core calls this from its single state-owning thread. Implementations
/// must never block on an unresponsive application window.
pub trait Platform {
    /// All connected monitors, primary first.
    fn monitors(&self) -> Result<Vec<MonitorInfo>>;

    /// All top-level windows, in z-order (topmost first).
    fn windows(&self) -> Result<Vec<WindowInfo>>;

    /// Fresh information about one window, or `None` if it is gone.
    fn window_info(&self, handle: WindowHandle) -> Option<WindowInfo>;

    /// The current foreground window, if any.
    fn foreground(&self) -> Option<WindowHandle>;

    /// Moves and resizes a batch of windows in one go. `rect` is the desired
    /// visible frame; the platform compensates for invisible borders.
    fn set_rects(&mut self, moves: &[(WindowHandle, Rect)]) -> Result<()>;

    fn show(&mut self, handle: WindowHandle) -> Result<()>;

    fn hide(&mut self, handle: WindowHandle) -> Result<()>;

    fn focus(&mut self, handle: WindowHandle) -> Result<()>;

    /// Politely asks a window to close (like clicking its close button).
    fn close(&mut self, handle: WindowHandle) -> Result<()>;

    /// The mouse cursor position in physical pixels.
    fn cursor_pos(&self) -> Option<(i32, i32)>;

    /// Launches a command line detached from the window manager.
    fn spawn(&mut self, command_line: &str) -> Result<()>;
}
