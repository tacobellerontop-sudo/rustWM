//! Win32 backend for rustWM.
//!
//! This crate only builds its real contents on Windows; on other targets it is
//! empty so the workspace still builds and tests everywhere.

#[cfg(windows)]
mod events;
#[cfg(windows)]
mod hidden;
#[cfg(windows)]
mod monitor;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use events::{EventThread, install_ctrl_handler};
#[cfg(windows)]
pub use hidden::restore_hidden_windows;
#[cfg(windows)]
pub use platform::{Win32Platform, enable_dpi_awareness};
