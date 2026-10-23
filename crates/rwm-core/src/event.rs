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
    /// A command string from a hotkey binding.
    Command(String),
    /// A command from an IPC client, which waits until request `id` has been
    /// handled before replying.
    Request {
        id: u64,
        command: String,
    },
    /// Time to check that recently moved windows took the size they were
    /// given. Sent by the daemon shortly after [`crate::Wm::take_verify_request`].
    Verify,
    /// Stop the window manager and restore all windows.
    Shutdown,
}
