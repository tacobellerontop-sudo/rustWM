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

/// How windows on inactive workspaces are taken off the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HideMethod {
    /// Move them outside every monitor. They keep their taskbar buttons,
    /// and clicking one switches to its workspace.
    #[default]
    Offscreen,
    /// Hide them (`SW_HIDE`); they leave the taskbar until shown again.
    Hide,
}

/// Behaviour the platform needs to know about, from the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlatformOptions {
    pub hide_method: HideMethod,
    /// How long window moves are animated; 0 moves them at once.
    pub animation_ms: u32,
}

/// The coloured frame drawn around the focused window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    pub window: WindowHandle,
    /// The window's visible frame; the border is drawn just inside it.
    pub rect: Rect,
    /// Thickness in physical pixels.
    pub width: i32,
    /// 0xRRGGBB.
    pub color: u32,
}

/// A tab bar over a stacked tile, listing the stack's windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stackbar {
    /// Where the bar goes, physical pixels.
    pub rect: Rect,
    /// The stack's windows and their titles, in order.
    pub tabs: Vec<(WindowHandle, String)>,
    /// Index of the shown window in `tabs`.
    pub active: usize,
    /// Whether the stack holds the focused window.
    pub focused: bool,
    /// Colours (0xRRGGBB): the shown tab, the others, and the text.
    pub active_color: u32,
    pub inactive_color: u32,
    pub text_color: u32,
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

    /// The smallest (width, height) the window lets itself be sized to, if
    /// it says.
    fn min_size(&self, _handle: WindowHandle) -> Option<(i32, i32)> {
        None
    }

    /// The current foreground window, if any.
    fn foreground(&self) -> Option<WindowHandle>;

    /// Moves and resizes a batch of windows in one go. `rect` is the desired
    /// visible frame; the platform compensates for invisible borders.
    fn set_rects(&mut self, moves: &[(WindowHandle, Rect)]) -> Result<()>;

    fn show(&mut self, handle: WindowHandle) -> Result<()>;

    fn hide(&mut self, handle: WindowHandle) -> Result<()>;

    fn focus(&mut self, handle: WindowHandle) -> Result<()>;

    /// Gives focus to the desktop, e.g. after switching to an empty workspace.
    fn focus_desktop(&mut self) -> Result<()>;

    /// Politely asks a window to close (like clicking its close button).
    fn close(&mut self, handle: WindowHandle) -> Result<()>;

    /// The mouse cursor position in physical pixels.
    fn cursor_pos(&self) -> Option<(i32, i32)>;

    /// Launches a command line detached from the window manager.
    fn spawn(&mut self, command_line: &str) -> Result<()>;

    /// Draws the focus border, or hides it with `None`.
    fn set_border(&mut self, border: Option<Border>);

    /// Applies config the platform cares about. Called at start and on reload.
    fn configure(&mut self, _options: PlatformOptions) {}

    /// Moves the mouse cursor (physical pixels), for `mouse_follows_focus`.
    fn set_cursor_pos(&mut self, _x: i32, _y: i32) -> Result<()> {
        Ok(())
    }

    /// Makes a window translucent: 255 is opaque (and undoes any change).
    fn set_opacity(&mut self, _handle: WindowHandle, _alpha: u8) -> Result<()> {
        Ok(())
    }

    /// Minimizes a window.
    fn minimize(&mut self, _handle: WindowHandle) -> Result<()> {
        Ok(())
    }

    /// The id of the process that owns a window.
    fn process_id(&self, _handle: WindowHandle) -> Option<u32> {
        None
    }

    /// The ids of a process's parent, grandparent and so on, nearest first.
    fn ancestor_pids(&self, _pid: u32) -> Vec<u32> {
        Vec::new()
    }

    /// Draws these stack tab bars, replacing any drawn before. Clicking a tab
    /// should send the command `focus-window <handle>`.
    fn set_stackbars(&mut self, _bars: &[Stackbar]) {}
}
