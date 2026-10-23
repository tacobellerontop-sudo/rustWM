//! Win32 backend for rustWM.
//!
//! This crate only builds its real contents on Windows; on other targets it is
//! empty so the workspace still builds and tests everywhere.

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::Win32Platform;
