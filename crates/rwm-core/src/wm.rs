use std::collections::BTreeSet;

use tracing::{debug, info, warn};

use crate::{Event, MonitorInfo, Platform, Rect, WindowHandle, layout};

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
    /// Managed windows in the order they were adopted.
    managed: Vec<WindowHandle>,
    /// Windows rustWM itself hid. They must be shown again on exit or crash,
    /// and their `WindowHidden` events must not be read as closes.
    hidden_by_us: BTreeSet<WindowHandle>,
    /// M1 debug mode: keep every managed window tiled in equal columns on the
    /// primary monitor. Replaced by real workspaces and layouts in M2.
    debug_tile: bool,
}

impl<P: Platform> Wm<P> {
    pub fn new(platform: P) -> Self {
        Self {
            platform,
            monitors: Vec::new(),
            managed: Vec::new(),
            hidden_by_us: BTreeSet::new(),
            debug_tile: false,
        }
    }

    /// Reads monitors and adopts every manageable window already on screen.
    pub fn start(&mut self) -> crate::platform::Result<()> {
        self.monitors = self.platform.monitors()?;
        for m in &self.monitors {
            info!(id = %m.id, work_area = ?m.work_area, scale = m.scale, "monitor");
        }
        for w in self.platform.windows()? {
            if w.manageable && !w.minimized {
                debug!(handle = ?w.handle, exe = %w.exe, title = %w.title, "adopting window");
                self.managed.push(w.handle);
            }
        }
        info!(count = self.managed.len(), "windows adopted");
        Ok(())
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        debug!(?event, "event");
        let changed = match event {
            Event::WindowShown(h) | Event::WindowRestored(h) => {
                !self.hidden_by_us.contains(&h) && self.manage(h)
            }
            Event::WindowHidden(h) => !self.hidden_by_us.contains(&h) && self.unmanage(h),
            Event::WindowDestroyed(h) => {
                self.hidden_by_us.remove(&h);
                self.unmanage(h)
            }
            Event::WindowMinimized(h) => self.unmanage(h),
            Event::DisplayChanged => {
                match self.platform.monitors() {
                    Ok(m) => self.monitors = m,
                    Err(e) => warn!(error = %e, "failed to re-read monitors"),
                }
                true
            }
            Event::Command(cmd) => self.run_command(&cmd),
            Event::Shutdown => {
                self.restore();
                return Flow::Exit;
            }
            // A dragged window snaps back into its slot.
            Event::WindowMoveEnded(h) => self.managed.contains(&h),
            Event::WindowFocused(_) | Event::WindowTitleChanged(_) => false,
        };
        if changed && self.debug_tile {
            self.debug_tile_all();
        }
        Flow::Continue
    }

    /// Runs a command string. Returns whether the window set changed.
    fn run_command(&mut self, cmd: &str) -> bool {
        match cmd.trim() {
            "debug-tile" => {
                self.debug_tile = true;
                true
            }
            "debug-untile" => {
                self.debug_tile = false;
                false
            }
            "restore" => {
                self.restore();
                false
            }
            other => {
                warn!(command = other, "unknown command");
                false
            }
        }
    }

    fn manage(&mut self, h: WindowHandle) -> bool {
        if self.managed.contains(&h) {
            return false;
        }
        match self.platform.window_info(h) {
            Some(w) if w.manageable && !w.minimized => {
                debug!(handle = ?h, exe = %w.exe, title = %w.title, "managing window");
                self.managed.push(h);
                true
            }
            _ => false,
        }
    }

    fn unmanage(&mut self, h: WindowHandle) -> bool {
        let before = self.managed.len();
        self.managed.retain(|x| *x != h);
        self.managed.len() != before
    }

    fn primary_work_area(&self) -> Option<Rect> {
        self.monitors
            .iter()
            .find(|m| m.primary)
            .or(self.monitors.first())
            .map(|m| m.work_area)
    }

    /// Tiles every managed window side by side on the primary monitor.
    pub fn debug_tile_all(&mut self) {
        let Some(area) = self.primary_work_area() else {
            warn!("no monitor to tile on");
            return;
        };
        let rects = layout::columns(self.managed.len(), area);
        let moves: Vec<_> = self.managed.iter().copied().zip(rects).collect();
        if let Err(e) = self.platform.set_rects(&moves) {
            warn!(error = %e, "set_rects failed");
        }
    }

    /// Hides a window and remembers that we did, so [`Wm::restore`] can undo it.
    pub fn hide(&mut self, handle: WindowHandle) {
        self.hidden_by_us.insert(handle);
        if let Err(e) = self.platform.hide(handle) {
            warn!(?handle, error = %e, "hide failed");
            self.hidden_by_us.remove(&handle);
        }
    }

    /// Shows every window rustWM hid. Called on exit and by the `restore` command.
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

    fn managed(wm: &Wm<FakePlatform>) -> Vec<isize> {
        wm.managed().map(|h| h.0).collect()
    }

    #[test]
    fn start_adopts_manageable_windows() {
        let mut p = FakePlatform::single_monitor();
        p.add_window(1, "a.exe", "a");
        let tool = p.add_window(2, "tool.exe", "tool");
        p.windows.get_mut(&tool).unwrap().manageable = false;

        let mut wm = Wm::new(p);
        wm.start().unwrap();

        assert_eq!(managed(&wm), vec![1]);
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
        assert_eq!(managed(&wm), vec![1, 2]);
    }

    #[test]
    fn foreign_hide_unmanages() {
        let mut wm = wm_with(2);
        wm.handle(Event::WindowHidden(WindowHandle(2)));
        assert_eq!(managed(&wm), vec![1]);
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
        assert_eq!(managed(&wm), vec![1, 9]);

        wm.platform_mut().close_window(h);
        wm.handle(Event::WindowDestroyed(h));
        assert_eq!(managed(&wm), vec![1]);
    }

    #[test]
    fn shown_twice_is_managed_once() {
        let mut wm = wm_with(1);
        wm.handle(Event::WindowShown(WindowHandle(1)));
        assert_eq!(managed(&wm), vec![1]);
    }

    #[test]
    fn minimize_and_restore_round_trip() {
        let mut wm = wm_with(2);
        wm.handle(Event::WindowMinimized(WindowHandle(1)));
        assert_eq!(managed(&wm), vec![2]);
        wm.handle(Event::WindowRestored(WindowHandle(1)));
        assert_eq!(managed(&wm), vec![2, 1]);
    }

    #[test]
    fn debug_tile_columns_and_retiles_on_open_and_close() {
        let mut wm = wm_with(2);
        wm.handle(Event::Command("debug-tile".into()));
        let p = wm.platform();
        assert_eq!(p.rect_of(WindowHandle(1)), Some(Rect::new(0, 0, 960, 1040)));
        assert_eq!(
            p.rect_of(WindowHandle(2)),
            Some(Rect::new(960, 0, 960, 1040))
        );

        let h = wm.platform_mut().add_window(3, "c.exe", "c");
        wm.handle(Event::WindowShown(h));
        assert_eq!(
            wm.platform().rect_of(h),
            Some(Rect::new(1280, 0, 640, 1040))
        );

        wm.platform_mut().close_window(WindowHandle(1));
        wm.handle(Event::WindowDestroyed(WindowHandle(1)));
        assert_eq!(
            wm.platform().rect_of(WindowHandle(2)),
            Some(Rect::new(0, 0, 960, 1040))
        );
    }

    #[test]
    fn minimized_windows_are_not_adopted() {
        let mut p = FakePlatform::single_monitor();
        p.add_window(1, "a.exe", "a");
        let min = p.add_window(2, "b.exe", "b");
        p.windows.get_mut(&min).unwrap().minimized = true;
        let mut wm = Wm::new(p);
        wm.start().unwrap();
        assert_eq!(managed(&wm), vec![1]);
    }

    #[test]
    fn dragged_window_snaps_back() {
        let mut wm = wm_with(2);
        wm.handle(Event::Command("debug-tile".into()));
        let slot = wm.platform().rect_of(WindowHandle(2));
        wm.platform_mut()
            .windows
            .get_mut(&WindowHandle(2))
            .unwrap()
            .rect = Rect::new(5, 5, 50, 50);
        wm.handle(Event::WindowMoveEnded(WindowHandle(2)));
        assert_eq!(wm.platform().rect_of(WindowHandle(2)), slot);
    }

    #[test]
    fn untile_stops_reacting() {
        let mut wm = wm_with(1);
        wm.handle(Event::Command("debug-tile".into()));
        wm.handle(Event::Command("debug-untile".into()));
        let calls = wm.platform().calls.len();
        let h = wm.platform_mut().add_window(2, "b.exe", "b");
        wm.handle(Event::WindowShown(h));
        assert_eq!(wm.platform().calls.len(), calls);
    }
}
