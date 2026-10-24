//! Workspaces, layouts, floating, fullscreen and rules, through the fake platform.

use rwm_core::fake::FakePlatform;
use rwm_core::layout::LayoutKind;
use rwm_core::rules::{Rule, RuleAction};
use rwm_core::wm::{Flow, Settings};
use rwm_core::{Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

fn settings() -> Settings {
    let mut s = Settings::default();
    s.gaps.inner = 0;
    s.gaps.outer = 0;
    s
}

fn wm_with_settings(n: isize, s: Settings) -> Wm<FakePlatform> {
    let mut p = FakePlatform::single_monitor();
    for i in 1..=n {
        p.add_window(i, "app.exe", &format!("window {i}"));
    }
    p.foreground = (n > 0).then_some(W(1));
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    wm
}

fn wm_with(n: isize) -> Wm<FakePlatform> {
    wm_with_settings(n, settings())
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) -> Flow {
    wm.handle(Event::Command(c.into()))
}

fn rect(wm: &Wm<FakePlatform>, id: isize) -> Rect {
    wm.platform().rect_of(W(id)).unwrap()
}

fn visible(wm: &Wm<FakePlatform>) -> Vec<isize> {
    wm.managed().map(|h| h.0).collect()
}

fn hidden(wm: &Wm<FakePlatform>) -> Vec<isize> {
    let mut v: Vec<_> = wm.platform().hidden.iter().map(|h| h.0).collect();
    v.sort();
    v
}

const FULL: Rect = Rect::new(0, 0, 1920, 1040);

#[test]
fn nine_workspaces_by_default() {
    let wm = wm_with(0);
    assert_eq!(wm.monitors()[0].workspaces.len(), 9);
}

#[test]
fn switching_workspaces_hides_and_shows() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "workspace 2");
    assert!(visible(&wm).is_empty());
    assert_eq!(hidden(&wm), vec![1, 2]);

    let h = wm.platform_mut().add_window(3, "new.exe", "new");
    wm.handle(Event::WindowShown(h));
    assert_eq!(visible(&wm), vec![3]);
    assert_eq!(rect(&wm, 3), FULL);

    cmd(&mut wm, "workspace 1");
    assert_eq!(visible(&wm), vec![1, 2]);
    assert_eq!(hidden(&wm), vec![3]);
    assert_eq!(wm.platform().foreground, Some(W(1)));
}

#[test]
fn our_own_hide_and_show_events_are_ignored() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "workspace 2");
    wm.handle(Event::WindowHidden(W(1)));
    cmd(&mut wm, "workspace 1");
    wm.handle(Event::WindowShown(W(1)));
    assert_eq!(visible(&wm), vec![1, 2]);
}

#[test]
fn move_to_workspace_sends_window_away() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "move-to-workspace 4");
    assert_eq!(visible(&wm), vec![2, 3]);
    assert_eq!(hidden(&wm), vec![1]);
    assert_eq!(wm.focused(), Some(W(2)));
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 960, 1040));

    cmd(&mut wm, "workspace 4");
    assert_eq!(visible(&wm), vec![1]);
    assert_eq!(rect(&wm, 1), FULL);
}

#[test]
fn unknown_workspace_is_ignored() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "workspace 42");
    cmd(&mut wm, "move-to-workspace nope");
    assert_eq!(visible(&wm), vec![1]);
}

#[test]
fn named_workspaces() {
    let s = Settings {
        workspaces: vec!["web".into(), "code".into()],
        ..settings()
    };
    let mut wm = wm_with_settings(1, s);
    cmd(&mut wm, "move-to-workspace code");
    assert_eq!(hidden(&wm), vec![1]);
    cmd(&mut wm, "workspace 2");
    assert_eq!(visible(&wm), vec![1]);
}

#[test]
fn quit_restores_windows_on_hidden_workspaces() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace 3");
    assert_eq!(cmd(&mut wm, "quit"), Flow::Exit);
    assert!(hidden(&wm).is_empty());
}

#[test]
fn closing_a_hidden_window_forgets_it() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace 3");
    wm.platform_mut().close_window(W(1));
    wm.handle(Event::WindowDestroyed(W(1)));
    assert_eq!(wm.all_windows().map(|h| h.0).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn layouts_switch_and_cycle() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "layout master");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
    assert_eq!(rect(&wm, 3), Rect::new(960, 520, 960, 520));

    cmd(&mut wm, "cycle-layout");
    assert_eq!(wm.monitors()[0].workspace().layout, LayoutKind::Monocle);
    for i in 1..=3 {
        assert_eq!(rect(&wm, i), FULL);
    }
    cmd(&mut wm, "cycle-layout");
    assert_eq!(wm.monitors()[0].workspace().layout, LayoutKind::Dwindle);
}

#[test]
fn monocle_directions_walk_the_list() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "layout monocle");
    cmd(&mut wm, "focus right");
    assert_eq!(wm.focused(), Some(W(2)));
    cmd(&mut wm, "focus up");
    assert_eq!(wm.focused(), Some(W(1)));
}

#[test]
fn layout_is_per_workspace() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "layout monocle");
    cmd(&mut wm, "workspace 2");
    assert_eq!(wm.monitors()[0].workspace().layout, LayoutKind::Dwindle);
}

#[test]
fn resize_and_reset() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "resize grow 0.1");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1152, 1040));
    assert_eq!(rect(&wm, 2), Rect::new(1152, 0, 768, 1040));
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "resize grow 0.2");
    assert_eq!(rect(&wm, 2), Rect::new(768, 0, 1152, 1040));
    cmd(&mut wm, "layout reset");
    assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 1040));
}

#[test]
fn toggle_float_centres_and_retiles() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "toggle-float");
    assert_eq!(rect(&wm, 2), FULL);
    assert_eq!(rect(&wm, 1), Rect::new(320, 173, 1280, 693));
    assert_eq!(wm.focused(), Some(W(1)));

    // Tiling it again appends it, like a newly opened window.
    cmd(&mut wm, "toggle-float");
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 960, 1040));
    assert_eq!(rect(&wm, 1), Rect::new(960, 0, 960, 1040));
}

#[test]
fn floating_windows_are_not_moved_by_layout() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "toggle-float");
    wm.platform_mut().place(W(1), Rect::new(10, 10, 300, 300));
    let h = wm.platform_mut().add_window(3, "c.exe", "c");
    wm.handle(Event::WindowShown(h));
    assert_eq!(rect(&wm, 1), Rect::new(10, 10, 300, 300));
    wm.handle(Event::WindowMoveEnded(W(1)));
    assert_eq!(rect(&wm, 1), Rect::new(10, 10, 300, 300));
}

#[test]
fn floating_windows_travel_with_their_workspace() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "toggle-float");
    cmd(&mut wm, "workspace 2");
    assert_eq!(hidden(&wm), vec![1, 2]);
    cmd(&mut wm, "workspace 1");
    assert!(hidden(&wm).is_empty());
}

#[test]
fn fullscreen_covers_the_monitor_and_toggles_back() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "toggle-fullscreen");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&wm, 2), FULL);
    cmd(&mut wm, "toggle-fullscreen");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
    assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 1040));
}

fn ruled(rules: Vec<Rule>) -> Wm<FakePlatform> {
    wm_with_settings(
        1,
        Settings {
            rules,
            ..settings()
        },
    )
}

#[test]
fn rule_float() {
    let mut wm = ruled(vec![
        Rule::new(Some("calc.exe"), None, None, vec![RuleAction::Float]).unwrap(),
    ]);
    let h = wm.platform_mut().add_window(2, "Calc.exe", "Calculator");
    wm.handle(Event::WindowShown(h));
    assert_eq!(rect(&wm, 1), FULL);
    assert_eq!(rect(&wm, 2), Rect::new(100, 100, 800, 600));
    assert_eq!(wm.focused(), Some(h));
}

#[test]
fn rule_ignore() {
    let mut wm = ruled(vec![
        Rule::new(
            None,
            None,
            Some("^Picture-in-Picture$"),
            vec![RuleAction::Ignore],
        )
        .unwrap(),
    ]);
    let h = wm
        .platform_mut()
        .add_window(2, "firefox.exe", "Picture-in-Picture");
    wm.handle(Event::WindowShown(h));
    wm.handle(Event::WindowTitleChanged(h));
    assert_eq!(visible(&wm), vec![1]);
}

#[test]
fn rule_workspace_opens_hidden_on_that_workspace() {
    let mut wm = ruled(vec![
        Rule::new(
            Some("Spotify.exe"),
            None,
            None,
            vec![RuleAction::Workspace("5".into())],
        )
        .unwrap(),
    ]);
    let h = wm.platform_mut().add_window(2, "Spotify.exe", "Spotify");
    wm.handle(Event::WindowShown(h));
    assert_eq!(visible(&wm), vec![1]);
    assert_eq!(hidden(&wm), vec![2]);
    assert_eq!(wm.focused(), Some(W(1)));
    cmd(&mut wm, "workspace 5");
    assert_eq!(visible(&wm), vec![2]);
}

#[test]
fn rules_apply_at_startup() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "a");
    p.add_window(2, "Spotify.exe", "Spotify");
    let s = Settings {
        rules: vec![
            Rule::new(
                Some("Spotify.exe"),
                None,
                None,
                vec![RuleAction::Workspace("2".into())],
            )
            .unwrap(),
        ],
        ..settings()
    };
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    assert_eq!(visible(&wm), vec![1]);
    assert_eq!(hidden(&wm), vec![2]);
}

#[test]
fn reload_command_asks_caller_and_settings_apply() {
    let mut wm = wm_with(2);
    assert_eq!(cmd(&mut wm, "reload"), Flow::Reload);
    let mut s = settings();
    s.gaps.outer = 10;
    s.gaps.inner = 0;
    s.gaps.smart = false;
    wm.set_settings(s);
    assert_eq!(rect(&wm, 1), Rect::new(10, 10, 950, 1020));
}

#[test]
fn shrinking_workspace_list_folds_windows_into_last() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace 9");
    let s = Settings {
        workspaces: vec!["1".into(), "2".into()],
        ..settings()
    };
    wm.set_settings(s);
    assert_eq!(wm.monitors()[0].workspaces.len(), 2);
    cmd(&mut wm, "workspace 2");
    assert_eq!(visible(&wm), vec![1]);
}

mod dual_monitor {
    use super::*;

    fn wm() -> Wm<FakePlatform> {
        let mut p = FakePlatform::dual_monitor();
        p.add_window(1, "a.exe", "a");
        let h = p.add_window(2, "b.exe", "b");
        p.place(h, Rect::new(2500, 100, 800, 600));
        p.foreground = Some(W(1));
        let mut wm = Wm::with_settings(p, settings());
        wm.start().unwrap();
        wm
    }

    #[test]
    fn workspaces_are_per_monitor() {
        let mut wm = wm();
        cmd(&mut wm, "workspace 2");
        assert_eq!(visible(&wm), vec![2]);
        assert_eq!(hidden(&wm), vec![1]);
    }

    #[test]
    fn focus_and_move_to_monitor() {
        let mut wm = wm();
        cmd(&mut wm, "focus-monitor right");
        assert_eq!(wm.focused(), Some(W(2)));
        assert_eq!(wm.focused_monitor(), 1);
        cmd(&mut wm, "move-to-monitor left");
        assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 1040));
        assert_eq!(wm.focused_monitor(), 0);
    }

    #[test]
    fn floating_window_moved_to_monitor_is_recentred() {
        let mut wm = wm();
        cmd(&mut wm, "toggle-float");
        cmd(&mut wm, "move-to-monitor right");
        let r = rect(&wm, 1);
        assert!(r.x >= 1920, "{r:?}");
    }
}
