use serde::{Deserialize, Serialize};

use crate::Rect;

/// An opaque OS window handle (an `HWND` on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowHandle(pub isize);

/// What the platform reports about a top-level window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub handle: WindowHandle,
    pub title: String,
    pub class: String,
    /// Executable file name, e.g. `firefox.exe`. Empty if it could not be read.
    pub exe: String,
    /// Visible frame of the window, without invisible DWM borders.
    pub rect: Rect,
    pub minimized: bool,
    /// The window has a sizing border. Windows without one (fixed-size
    /// launchers, dialogs) are floated instead of tiled.
    pub resizable: bool,
    /// The window passed the platform's "is this a normal app window" filter
    /// (visible, not a tool window, not owned, not cloaked, non-zero size).
    pub manageable: bool,
}
