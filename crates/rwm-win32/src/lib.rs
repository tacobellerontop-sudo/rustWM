//! Win32 backend for rustWM.
//!
//! This crate only builds its real contents on Windows; on other targets it is
//! empty so the workspace still builds and tests everywhere.

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
mod events;
#[cfg(windows)]
mod hidden;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod monitor;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
pub mod user_path;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use events::{EventThread, install_ctrl_handler};
#[cfg(windows)]
pub use hidden::restore_hidden_windows;
#[cfg(windows)]
pub use hotkeys::{set_bindings, set_mode};
#[cfg(windows)]
pub use platform::{Win32Platform, enable_dpi_awareness};
#[cfg(windows)]
pub use tray::{notify, set_config_path};
