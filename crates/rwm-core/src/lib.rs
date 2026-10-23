//! Platform-independent core of rustWM.
//!
//! Everything in this crate is pure Rust with no Win32 calls. The window
//! manager talks to the OS only through the [`Platform`] trait, so the whole
//! core can be driven by [`fake::FakePlatform`] in tests on any OS.

pub mod cmdline;
pub mod event;
pub mod geometry;
pub mod layout;
pub mod platform;
pub mod window;
pub mod wm;

#[cfg(feature = "fake")]
pub mod fake;

pub use event::Event;
pub use geometry::Rect;
pub use platform::{MonitorInfo, Platform, PlatformError};
pub use window::{WindowHandle, WindowInfo};
pub use wm::Wm;
