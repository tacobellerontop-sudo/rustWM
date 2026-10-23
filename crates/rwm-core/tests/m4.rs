//! Special workspaces, binding modes, drag-to-swap and state snapshots.

use rwm_core::fake::{Call, FakePlatform};
use rwm_core::rules::{Rule, RuleAction};
use rwm_core::wm::Settings;
use rwm_core::{Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

fn settings() -> Settings {
    let mut s = Settings::default();
    s.gaps.inner = 0;
    s.gaps.outer = 0;
    s
}

fn wm_with(n: isize) -> Wm<FakePlatform> {
    let mut p = FakePlatform::single_monitor();
    for i in 1..=n {
        p.add_window(i, "app.exe", &format!("window {i}"));
    }
    p.foreground = (n > 0).then_some(W(1));
    let mut wm = Wm::with_settings(p, settings());
    wm.start().unwrap();
    wm
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) {
    wm.handle(Event::Command(c.into()));
}

fn rect(wm: &Wm<FakePlatform>, id: isize) -> Rect {
    wm.platform().rect_of(W(id)).unwrap()
}

fn hidden(wm: &Wm<FakePlatform>) -> Vec<isize> {
    let mut v: Vec<_> = wm.platform().hidden.iter().map(|h| h.0).collect();
    v.sort();
    v
}

fn spawns(wm: &Wm<FakePlatform>) -> Vec<String> {
    wm.platform()
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Spawn(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// 90% of the 1920x1040 work area, centred.
const OVERLAY: Rect = Rect::new(96, 52, 1728, 936);

#[test]
fn empty_special_spawns_terminal_and_adopts_it() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "toggle-special");
    assert_eq!(spawns(&wm), vec!["wt.exe"]);

    let h = wm
        .platform_mut()
        .add_window(7, "WindowsTerminal.exe", "Terminal");
    wm.handle(Event::WindowShown(h));
    assert_eq!(rect(&wm, 7), OVERLAY);
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
    assert_eq!(wm.focused(), Some(h));

    cmd(&mut wm, "toggle-special");
    assert_eq!(hidden(&wm), vec![7]);
    assert_eq!(wm.focused(), Some(W(1)));

    cmd(&mut wm, "toggle-special scratch");
    assert!(hidden(&wm).is_empty());
    assert_eq!(wm.platform().foreground, Some(h));
    assert_eq!(
        spawns(&wm).len(),
        1,
        "no second terminal when it has windows"
    );
}

#[test]
fn move_to_special_and_back() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace special:scratch");
    assert_eq!(hidden(&wm), vec![1]);
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));

    cmd(&mut wm, "toggle-special");
    assert_eq!(rect(&wm, 1), OVERLAY);
    cmd(&mut wm, "move-to-workspace 1");
    assert!(hidden(&wm).is_empty());
    assert_eq!(rect(&wm, 1), Rect::new(960, 0, 960, 1040));
    assert_eq!(wm.focused(), Some(W(1)));
}

#[test]
fn rule_can_target_special() {
    let mut s = settings();
    s.rules = vec![
        Rule::new(
            Some("Spotify.exe"),
            None,
            None,
            vec![RuleAction::Workspace("special:music".into())],
        )
        .unwrap(),
    ];
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "a");
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    let h = wm.platform_mut().add_window(2, "Spotify.exe", "Spotify");
    wm.handle(Event::WindowShown(h));
    assert_eq!(hidden(&wm), vec![2]);
    cmd(&mut wm, "toggle-special music");
    assert!(hidden(&wm).is_empty());
    assert_eq!(rect(&wm, 2), OVERLAY);
}

#[test]
fn switching_specials_hides_the_other() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace special:a");
    cmd(&mut wm, "move-to-workspace special:b");
    cmd(&mut wm, "toggle-special a");
    assert_eq!(hidden(&wm), vec![2]);
    cmd(&mut wm, "toggle-special b");
    assert_eq!(hidden(&wm), vec![1]);
}

#[test]
fn focus_outside_special_returns_new_windows_to_the_workspace() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "move-to-workspace special:scratch");
    let h = wm.platform_mut().add_window(2, "b.exe", "b");
    wm.handle(Event::WindowShown(h));
    cmd(&mut wm, "toggle-special");
    wm.handle(Event::WindowFocused(W(2)));
    let n = wm.platform_mut().add_window(3, "c.exe", "c");
    wm.handle(Event::WindowShown(n));
    assert_eq!(rect(&wm, 3), Rect::new(960, 0, 960, 1040));
}

#[test]
fn quit_restores_special_windows() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace special:scratch");
    cmd(&mut wm, "quit");
    assert!(hidden(&wm).is_empty());
}

#[test]
fn mode_command_changes_mode() {
    let mut wm = wm_with(1);
    assert_eq!(wm.mode(), "default");
    cmd(&mut wm, "mode resize");
    assert_eq!(wm.mode(), "resize");
    assert_eq!(wm.snapshot().mode, "resize");
    cmd(&mut wm, "mode default");
    assert_eq!(wm.mode(), "default");
}

#[test]
fn dropping_a_window_on_another_swaps_them() {
    let mut wm = wm_with(3);
    // 1 = left half, 2 = top right, 3 = bottom right. Drag 1 onto 3.
    wm.platform_mut().cursor = Some((1500, 800));
    wm.platform_mut()
        .place(W(1), Rect::new(1300, 600, 500, 400));
    wm.handle(Event::WindowMoveEnded(W(1)));
    assert_eq!(rect(&wm, 3), Rect::new(0, 0, 960, 1040));
    assert_eq!(rect(&wm, 1), Rect::new(960, 520, 960, 520));
}

#[test]
fn dropping_on_empty_space_snaps_back() {
    let mut wm = wm_with(2);
    wm.platform_mut().cursor = Some((100, 100));
    wm.platform_mut().place(W(1), Rect::new(0, 0, 200, 200));
    wm.handle(Event::WindowMoveEnded(W(1)));
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
}

#[test]
fn snapshot_describes_everything() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace 3");
    let s = wm.snapshot();
    assert_eq!(s.focused_window.as_ref().map(|w| w.handle), Some(2));
    assert_eq!(s.focused_window.as_ref().unwrap().title, "window 2");
    let m = &s.monitors[0];
    assert_eq!(m.active_workspace, "1");
    assert_eq!(m.workspaces.len(), 9);
    assert_eq!(m.workspaces[2].windows[0].handle, 1);
    assert_eq!(m.workspaces[2].windows[0].exe, "app.exe");
}

#[test]
fn snapshot_tracks_title_changes() {
    let mut wm = wm_with(1);
    wm.platform_mut().windows.get_mut(&W(1)).unwrap().title = "renamed".into();
    wm.handle(Event::WindowTitleChanged(W(1)));
    assert_eq!(
        wm.snapshot().monitors[0].workspaces[0].windows[0].title,
        "renamed"
    );
}

#[test]
fn state_changes_name_topics() {
    let mut wm = wm_with(2);
    let a = wm.snapshot();
    cmd(&mut wm, "focus right");
    let b = wm.snapshot();
    assert_eq!(b.changes(&a), vec!["focus"]);

    cmd(&mut wm, "workspace 2");
    let c = wm.snapshot();
    assert_eq!(c.changes(&b), vec!["focus", "workspace"]);

    let h = wm.platform_mut().add_window(9, "n.exe", "n");
    wm.handle(Event::WindowShown(h));
    let d = wm.snapshot();
    assert!(d.changes(&c).contains(&"window"));
    assert!(d.changes(&d).is_empty());
}

#[test]
fn ipc_requests_run_like_commands() {
    let mut wm = wm_with(2);
    wm.handle(Event::Request {
        id: 1,
        command: "focus right".into(),
    });
    assert_eq!(wm.focused(), Some(W(2)));
}
