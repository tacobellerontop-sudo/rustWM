//! Workspaces across two monitors: per-monitor and global modes.

use rwm_core::fake::FakePlatform;
use rwm_core::wm::Settings;
use rwm_core::workspace::WorkspaceMode;
use rwm_core::{Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

/// Window 1 on the left monitor, window 2 on the right one; 1 is focused.
fn dual(mode: WorkspaceMode) -> Wm<FakePlatform> {
    let mut p = FakePlatform::dual_monitor();
    p.add_window(1, "a.exe", "left");
    p.add_window(2, "b.exe", "right");
    p.place(W(2), Rect::new(2100, 100, 800, 600));
    p.foreground = Some(W(1));
    let s = Settings {
        workspace_mode: mode,
        ..Settings::default()
    };
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    wm
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) {
    wm.handle(Event::Command(c.into()));
}

fn hidden(wm: &Wm<FakePlatform>) -> Vec<isize> {
    let mut v: Vec<_> = wm.platform().hidden.iter().map(|h| h.0).collect();
    v.sort();
    v
}

fn active(wm: &Wm<FakePlatform>) -> Vec<String> {
    wm.snapshot()
        .monitors
        .into_iter()
        .map(|m| m.active_workspace)
        .collect()
}

#[test]
fn per_monitor_switches_only_the_focused_monitor() {
    let mut wm = dual(WorkspaceMode::PerMonitor);
    assert_eq!(wm.platform().rect_of(W(2)).unwrap().x, 1920);
    cmd(&mut wm, "workspace 2");
    assert_eq!(hidden(&wm), vec![1]);
    assert_eq!(active(&wm), vec!["2", "1"]);
}

#[test]
fn clicking_the_desktop_focuses_the_monitor_under_the_mouse() {
    let mut wm = dual(WorkspaceMode::PerMonitor);
    // Click the right monitor's empty desktop (an unmanaged window).
    wm.platform_mut().cursor = Some((3000, 500));
    wm.handle(Event::WindowFocused(W(999)));
    assert_eq!(wm.focused_monitor(), 1);
    cmd(&mut wm, "workspace 3");
    assert_eq!(hidden(&wm), vec![2]);
    assert_eq!(active(&wm), vec!["1", "3"]);
    // New windows open on that monitor too.
    wm.platform_mut().add_window(3, "c.exe", "new");
    wm.handle(Event::WindowShown(W(3)));
    assert!(wm.platform().rect_of(W(3)).unwrap().x >= 1920);
}

#[test]
fn global_switches_every_monitor() {
    let mut wm = dual(WorkspaceMode::Global);
    cmd(&mut wm, "workspace 2");
    assert_eq!(hidden(&wm), vec![1, 2]);
    assert_eq!(active(&wm), vec!["2", "2"]);
    cmd(&mut wm, "workspace 1");
    assert!(hidden(&wm).is_empty());
    assert_eq!(active(&wm), vec!["1", "1"]);
}

#[test]
fn global_moves_keep_windows_on_their_monitor() {
    let mut wm = dual(WorkspaceMode::Global);
    wm.handle(Event::WindowFocused(W(2)));
    cmd(&mut wm, "move-to-workspace 3");
    assert_eq!(hidden(&wm), vec![2]);
    cmd(&mut wm, "workspace 3");
    assert_eq!(hidden(&wm), vec![1]);
    assert!(wm.platform().rect_of(W(2)).unwrap().x >= 1920);
    // The left monitor's half of workspace 3 is empty, so the right one has focus.
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn switching_to_global_lines_monitors_up() {
    let mut wm = dual(WorkspaceMode::PerMonitor);
    cmd(&mut wm, "workspace 4");
    assert_eq!(active(&wm), vec!["4", "1"]);
    cmd(&mut wm, "workspace-mode toggle");
    assert_eq!(active(&wm), vec!["4", "4"]);
    assert_eq!(hidden(&wm), vec![1, 2]);
    cmd(&mut wm, "workspace-mode per-monitor");
    cmd(&mut wm, "workspace 1");
    assert_eq!(active(&wm), vec!["1", "4"]);
}
