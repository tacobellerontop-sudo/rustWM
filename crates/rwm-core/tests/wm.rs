//! Window-manager behaviour, driven end to end through the fake platform.

use rwm_core::fake::{Call, FakePlatform};
use rwm_core::wm::{Flow, Settings};
use rwm_core::{Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

fn no_gaps() -> Settings {
    let mut s = Settings::default();
    s.gaps.inner = 0;
    s.gaps.outer = 0;
    s
}

/// A single 1920x1040 work area with windows 1..=n, window 1 focused.
fn wm_with(n: isize) -> Wm<FakePlatform> {
    let mut p = FakePlatform::single_monitor();
    for i in 1..=n {
        p.add_window(i, "app.exe", &format!("window {i}"));
    }
    p.foreground = (n > 0).then_some(W(1));
    let mut wm = Wm::with_settings(p, no_gaps());
    wm.start().unwrap();
    wm
}

fn order(wm: &Wm<FakePlatform>) -> Vec<isize> {
    wm.managed().map(|h| h.0).collect()
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) -> Flow {
    wm.handle(Event::Command(c.into()))
}

fn rect(wm: &Wm<FakePlatform>, id: isize) -> Rect {
    wm.platform().rect_of(W(id)).unwrap()
}

#[test]
fn start_tiles_existing_windows_with_dwindle() {
    let wm = wm_with(3);
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
    assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 520));
    assert_eq!(rect(&wm, 3), Rect::new(960, 520, 960, 520));
}

#[test]
fn start_skips_unmanageable_and_minimized() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "a");
    let tool = p.add_window(2, "tool.exe", "tool");
    p.windows.get_mut(&tool).unwrap().manageable = false;
    let min = p.add_window(3, "b.exe", "b");
    p.windows.get_mut(&min).unwrap().minimized = true;
    let mut wm = Wm::new(p);
    wm.start().unwrap();
    assert_eq!(order(&wm), vec![1]);
}

#[test]
fn new_window_goes_after_focused_and_reflows() {
    let mut wm = wm_with(2);
    let h = wm.platform_mut().add_window(9, "new.exe", "new");
    wm.handle(Event::WindowShown(h));
    assert_eq!(order(&wm), vec![1, 9, 2]);
    assert_eq!(wm.focused(), Some(h));
    assert_eq!(rect(&wm, 9), Rect::new(960, 0, 960, 520));
    assert_eq!(rect(&wm, 2), Rect::new(960, 520, 960, 520));
}

#[test]
fn closing_a_window_reflows() {
    let mut wm = wm_with(2);
    wm.platform_mut().close_window(W(2));
    wm.handle(Event::WindowDestroyed(W(2)));
    assert_eq!(order(&wm), vec![1]);
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

#[test]
fn shown_twice_and_title_changes_manage_once() {
    let mut wm = wm_with(1);
    wm.handle(Event::WindowShown(W(1)));
    wm.handle(Event::WindowTitleChanged(W(1)));
    assert_eq!(order(&wm), vec![1]);
}

#[test]
fn window_that_gets_a_title_later_is_managed() {
    let mut wm = wm_with(1);
    let h = wm.platform_mut().add_window(5, "slow.exe", "");
    wm.platform_mut().windows.get_mut(&h).unwrap().manageable = false;
    wm.handle(Event::WindowShown(h));
    assert_eq!(order(&wm), vec![1]);

    wm.platform_mut().windows.get_mut(&h).unwrap().manageable = true;
    wm.handle(Event::WindowTitleChanged(h));
    assert_eq!(order(&wm), vec![1, 5]);
}

#[test]
fn minimize_and_restore_round_trip() {
    let mut wm = wm_with(2);
    wm.handle(Event::WindowMinimized(W(2)));
    assert_eq!(order(&wm), vec![1]);
    wm.handle(Event::WindowRestored(W(2)));
    assert_eq!(order(&wm), vec![1, 2]);
}

#[test]
fn own_hide_is_not_a_close_and_shutdown_restores() {
    let mut wm = wm_with(2);
    wm.hide(W(1));
    wm.handle(Event::WindowHidden(W(1)));
    assert!(order(&wm).contains(&1));
    assert_eq!(wm.handle(Event::Shutdown), Flow::Exit);
    assert!(wm.platform().hidden.is_empty());
}

#[test]
fn foreign_hide_unmanages() {
    let mut wm = wm_with(2);
    wm.handle(Event::WindowHidden(W(2)));
    assert_eq!(order(&wm), vec![1]);
}

#[test]
fn focus_events_track_the_focused_window() {
    let mut wm = wm_with(3);
    wm.handle(Event::WindowFocused(W(3)));
    assert_eq!(wm.focused(), Some(W(3)));
    // Focus on an unmanaged window leaves our focus alone.
    wm.handle(Event::WindowFocused(W(77)));
    assert_eq!(wm.focused(), Some(W(3)));
}

#[test]
fn geometric_focus() {
    // 1 = left half, 2 = top right, 3 = bottom right.
    let mut wm = wm_with(3);
    cmd(&mut wm, "focus right");
    assert_eq!(wm.focused(), Some(W(2)));
    cmd(&mut wm, "focus down");
    assert_eq!(wm.focused(), Some(W(3)));
    cmd(&mut wm, "focus left");
    assert_eq!(wm.focused(), Some(W(1)));
    // Nothing further left: focus stays.
    cmd(&mut wm, "focus left");
    assert_eq!(wm.focused(), Some(W(1)));
    assert_eq!(wm.platform().foreground, Some(W(1)));
}

#[test]
fn focus_next_prev_wraps() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "focus prev");
    assert_eq!(wm.focused(), Some(W(3)));
    cmd(&mut wm, "focus next");
    assert_eq!(wm.focused(), Some(W(1)));
}

#[test]
fn move_swaps_with_geometric_neighbour() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "move right");
    assert_eq!(order(&wm), vec![2, 1, 3]);
    assert_eq!(wm.focused(), Some(W(1)));
    assert_eq!(rect(&wm, 1), Rect::new(960, 0, 960, 520));
}

#[test]
fn promote_swaps_with_first_then_back() {
    let mut wm = wm_with(3);
    wm.handle(Event::WindowFocused(W(3)));
    cmd(&mut wm, "promote");
    assert_eq!(order(&wm), vec![3, 2, 1]);
    cmd(&mut wm, "promote");
    assert_eq!(order(&wm), vec![2, 3, 1]);
}

#[test]
fn close_asks_the_focused_window() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "close");
    assert!(wm.platform().calls.contains(&Call::Close(W(1))));
}

#[test]
fn terminal_uses_setting_or_falls_back() {
    let mut wm = wm_with(1);
    wm.platform_mut().failing_spawns.push("wt.exe".into());
    cmd(&mut wm, "terminal");
    let spawns: Vec<_> = wm
        .platform()
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Spawn(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(spawns, vec!["wt.exe", "cmd.exe"]);

    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "a");
    let settings = Settings {
        terminal: Some("wezterm-gui.exe".into()),
        ..Settings::default()
    };
    let mut wm = Wm::with_settings(p, settings);
    wm.start().unwrap();
    cmd(&mut wm, "terminal");
    assert!(
        wm.platform()
            .calls
            .contains(&Call::Spawn("wezterm-gui.exe".into()))
    );
}

#[test]
fn exec_spawns_command_line() {
    let mut wm = wm_with(0);
    cmd(&mut wm, "exec notepad.exe C:\\notes.txt");
    assert_eq!(
        wm.platform().calls,
        vec![Call::Spawn("notepad.exe C:\\notes.txt".into())]
    );
}

#[test]
fn quit_exits_and_bad_commands_are_ignored() {
    let mut wm = wm_with(1);
    assert_eq!(cmd(&mut wm, "fly away"), Flow::Continue);
    assert_eq!(cmd(&mut wm, "quit"), Flow::Exit);
}

#[test]
fn unchanged_windows_are_not_moved_again() {
    let mut wm = wm_with(2);
    let before = wm.platform().calls.len();
    cmd(&mut wm, "focus right");
    let moves = wm.platform().calls[before..]
        .iter()
        .filter(|c| matches!(c, Call::SetRects(_)))
        .count();
    assert_eq!(moves, 0);
}

#[test]
fn dragged_window_snaps_back() {
    let mut wm = wm_with(2);
    let slot = rect(&wm, 2);
    wm.platform_mut()
        .place(W(2), Rect::new(1000, 300, slot.w, slot.h));
    wm.handle(Event::WindowMoveEnded(W(2)));
    assert_eq!(rect(&wm, 2), slot);
}

#[test]
fn retile_reapplies_everything() {
    let mut wm = wm_with(1);
    wm.platform_mut().place(W(1), Rect::new(5, 5, 5, 5));
    cmd(&mut wm, "retile");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

#[test]
fn gaps_are_applied() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "a");
    p.add_window(2, "b.exe", "b");
    let mut wm = Wm::new(p); // default gaps: 8 inner, 8 outer
    wm.start().unwrap();
    assert_eq!(rect(&wm, 1), Rect::new(8, 8, 948, 1024));
    assert_eq!(rect(&wm, 2), Rect::new(964, 8, 948, 1024));
}

mod dual_monitor {
    use super::*;

    /// Windows 1 and 2 on the left monitor, 3 on the right one.
    fn wm() -> Wm<FakePlatform> {
        let mut p = FakePlatform::dual_monitor();
        p.add_window(1, "a.exe", "a");
        p.add_window(2, "b.exe", "b");
        let h = p.add_window(3, "c.exe", "c");
        p.place(h, Rect::new(2500, 100, 800, 600));
        p.foreground = Some(W(1));
        let mut wm = Wm::with_settings(p, no_gaps());
        wm.start().unwrap();
        wm
    }

    #[test]
    fn windows_are_adopted_on_their_monitor() {
        let wm = wm();
        assert_eq!(rect(&wm, 3), Rect::new(1920, 0, 1920, 1080));
        assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 1040));
    }

    #[test]
    fn focus_crosses_monitors() {
        let mut wm = wm();
        cmd(&mut wm, "focus right");
        cmd(&mut wm, "focus right");
        assert_eq!(wm.focused(), Some(W(3)));
        cmd(&mut wm, "focus left");
        assert_eq!(wm.focused(), Some(W(2)));
    }

    #[test]
    fn new_windows_open_on_the_focused_monitor() {
        let mut wm = wm();
        cmd(&mut wm, "focus right");
        cmd(&mut wm, "focus right");
        let h = wm.platform_mut().add_window(9, "n.exe", "n");
        wm.handle(Event::WindowShown(h));
        assert_eq!(rect(&wm, 9), Rect::new(2880, 0, 960, 1080));
    }

    #[test]
    fn move_across_monitors_transfers() {
        let mut wm = wm();
        wm.handle(Event::WindowFocused(W(2)));
        cmd(&mut wm, "move right");
        assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
        assert_eq!(rect(&wm, 3), Rect::new(1920, 0, 960, 1080));
        assert_eq!(rect(&wm, 2), Rect::new(2880, 0, 960, 1080));
        assert_eq!(wm.focused(), Some(W(2)));
    }

    #[test]
    fn move_onto_empty_monitor() {
        let mut p = FakePlatform::dual_monitor();
        p.add_window(1, "a.exe", "a");
        p.add_window(2, "b.exe", "b");
        p.foreground = Some(W(2));
        let mut wm = Wm::with_settings(p, no_gaps());
        wm.start().unwrap();
        cmd(&mut wm, "move right");
        assert_eq!(rect(&wm, 2), Rect::new(1920, 0, 1920, 1080));
        assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
    }

    #[test]
    fn drop_on_other_monitor_transfers() {
        let mut wm = wm();
        let size = rect(&wm, 1);
        wm.platform_mut()
            .place(W(1), Rect::new(3000, 500, size.w, size.h));
        wm.handle(Event::WindowMoveEnded(W(1)));
        assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));
        // Dropped onto window 3, so it takes 3's slot and 3 moves along.
        assert_eq!(rect(&wm, 1).x, 1920);
        assert_eq!(rect(&wm, 3).x, 2880);
    }

    #[test]
    fn unplugged_monitor_windows_move_to_primary() {
        let mut wm = wm();
        wm.platform_mut().monitors.pop();
        wm.handle(Event::DisplayChanged);
        assert_eq!(order(&wm), vec![1, 2, 3]);
        assert!(rect(&wm, 3).right() <= 1920);
    }
}
