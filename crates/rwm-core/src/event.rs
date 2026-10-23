use crate::WindowHandle;

/// Everything that can wake the core thread.
///
/// The Win32, hotkey, IPC and config threads only translate what they see
/// into these; all decisions happen in [`crate::Wm::handle`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A window became visible (shown or created).
    WindowShown(WindowHandle),
    /// A window was hidden. May be our own hide, which the core filters out.
    WindowHidden(WindowHandle),
    WindowDestroyed(WindowHandle),
    WindowFocused(WindowHandle),
    WindowMinimized(WindowHandle),
    WindowRestored(WindowHandle),
    /// The user finished dragging or resizing a window.
    WindowMoveEnded(WindowHandle),
    WindowTitleChanged(WindowHandle),
    /// Monitors were added, removed or rearranged.
    DisplayChanged,
    /// A command string, from a hotkey binding or the CLI.
    Command(String),
    /// Stop the window manager and restore all windows.
    Shutdown,
}
