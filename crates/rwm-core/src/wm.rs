use std::collections::BTreeSet;

use tracing::{debug, info, warn};

use crate::{Event, MonitorInfo, Platform, WindowHandle};

/// Whether the event loop should keep running after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

/// The window manager: owns all state and is the only thing that calls into
/// the [`Platform`].
pub struct Wm<P: Platform> {
    platform: P,
    monitors: Vec<MonitorInfo>,
    managed: BTreeSet<WindowHandle>,
    /// Windows rustWM itself hid. They must be shown again on exit or crash,
    /// and their `WindowHidden` events must not be read as closes.
    hidden_by_us: BTreeSet<WindowHandle>,
}

impl<P: Platform> Wm<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            monitors: Vec::new(),
            managed: BTreeSet::new(),
            hidden_by_us: BTreeSet::new(),
        }
    }

    /// Reads monitors and adopts every manageable window already on screen.
    pub fn start(&mut self) -> crate::platform::Result<()> {
        self.monitors = self.platform.monitors()?;
        info!(count = self.monitors.len(), "monitors detected");
        for w in self.platform.windows()? {
            if w.manageable {
                debug!(handle = ?w.handle, exe = %w.exe, title = %w.title, "adopting window");
                self.managed.insert(w.handle);
            }
        }
        info!(count = self.managed.len(), "windows adopted");
        Ok(())
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        debug!(?event, "event");
        match event {
            Event::WindowShown(h) => {
                if self.hidden_by_us.contains(&h) {
                    return Flow::Continue;
                }
                if self.platform.window_info(h).is_some_and(|w| w.manageable) {
                    self.managed.insert(h);
                }
            }
            Event::WindowHidden(h) => {
                if !self.hidden_by_us.contains(&h) {
                    self.managed.remove(&h);
                }
            }
            Event::WindowDestroyed(h) => {
                self.managed.remove(&h);
                self.hidden_by_us.remove(&h);
            }
            Event::DisplayChanged => match self.platform.monitors() {
                Ok(m) => self.monitors = m,
                Err(e) => warn!(error = %e, "failed to re-read monitors"),
            },
            Event::Shutdown => {
                self.restore();
                return Flow::Exit;
            }
            Event::WindowFocused(_)
            | Event::WindowMinimized(_)
            | Event::WindowRestored(_)
            | Event::WindowMoveEnded(_)
            | Event::WindowTitleChanged(_)
            | Event::Command(_) => {}
        }
        Flow::Continue
    }

    /// Hides a window and remembers that we did, so [`Wm::restore`] can undo it.
    pub fn hide(&mut self, handle: WindowHandle) {
        self.hidden_by_us.insert(handle);
        if let Err(e) = self.platform.hide(handle) {
            warn!(?handle, error = %e, "hide failed");
            self.hidden_by_us.remove(&handle);
        }
    }

    /// Shows every window rustWM hid. Called on exit and from the panic hook.
    pub fn restore(&mut self) {
        for handle in std::mem::take(&mut self.hidden_by_us) {
            if let Err(e) = self.platform.show(handle) {
                warn!(?handle, error = %e, "restore failed");
            }
        }
    }

    pub fn managed(&self) -> impl Iterator<Item = WindowHandle> + '_ {
        self.managed.iter().copied()
    }

    pub fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    pub fn platform(&self) -> &P {
        &self.platform
    }

    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakePlatform;

    fn wm_with(n: isize) -> Wm<FakePlatform> {
        let mut p = FakePlatform::single_monitor();
        for i in 1..=n {
            p.add_window(i, "app.exe", &format!("window {i}"));
        }
        let mut wm = Wm::new(p);
        wm.start().unwrap();
        wm
    }

    #[test]
    fn start_adopts_manageable_windows() {
        let mut p = FakePlatform::single_monitor();
        p.add_window(1, "a.exe", "a");
        let tool = p.add_window(2, "tool.exe", "tool");
        p.windows.get_mut(&tool).unwrap().manageable = false;

        let mut wm = Wm::new(p);
        wm.start().unwrap();

        assert_eq!(wm.managed().collect::<Vec<_>>(), vec![WindowHandle(1)]);
        assert_eq!(wm.monitors().len(), 1);
    }

    #[test]
    fn own_hide_is_not_a_close() {
        let mut wm = wm_with(2);
        wm.hide(WindowHandle(1));
        assert_eq!(
            wm.handle(Event::WindowHidden(WindowHandle(1))),
            Flow::Continue
        );
        assert!(wm.managed().any(|h| h == WindowHandle(1)));
    }

    #[test]
    fn foreign_hide_unmanages() {
        let mut wm = wm_with(2);
        wm.handle(Event::WindowHidden(WindowHandle(2)));
        assert!(!wm.managed().any(|h| h == WindowHandle(2)));
    }

    #[test]
    fn shutdown_restores_hidden_windows() {
        let mut wm = wm_with(3);
        wm.hide(WindowHandle(1));
        wm.hide(WindowHandle(3));
        assert!(wm.platform().is_hidden(WindowHandle(1)));

        assert_eq!(wm.handle(Event::Shutdown), Flow::Exit);

        assert!(wm.platform().hidden.is_empty());
    }

    #[test]
    fn new_window_is_managed_and_closed_window_dropped() {
        let mut wm = wm_with(1);
        let h = wm.platform_mut().add_window(9, "new.exe", "new");
        wm.handle(Event::WindowShown(h));
        assert!(wm.managed().any(|x| x == h));

        wm.platform_mut().close_window(h);
        wm.handle(Event::WindowDestroyed(h));
        assert!(!wm.managed().any(|x| x == h));
    }
}
