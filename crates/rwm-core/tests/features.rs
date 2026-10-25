//! komorebi- and Hyprland-style features: stacks, pinning, maximize, focus
//! history, workspace navigation, extra layouts, keyboard resizing, pause,
//! manage/unmanage, mouse follows focus and inactive opacity.

use rwm_core::fake::{Call, FakePlatform};
use rwm_core::layout::{Gaps, LayoutKind, Side};
use rwm_core::platform::HideMethod;
use rwm_core::rules::{Rule, RuleAction};
use rwm_core::wm::{Offsets, Settings, WorkspaceOverride};
use rwm_core::{Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

fn no_gaps() -> Settings {
    let mut s = Settings::default();
    s.gaps.inner = 0;
    s.gaps.outer = 0;
    s
}

fn wm_on(mut p: FakePlatform, n: isize, settings: Settings) -> Wm<FakePlatform> {
    for i in 1..=n {
        p.add_window(i, "app.exe", &format!("window {i}"));
    }
    p.foreground = (n > 0).then_some(W(1));
    let mut wm = Wm::with_settings(p, settings);
    wm.start().unwrap();
    wm
}

fn wm_with(n: isize) -> Wm<FakePlatform> {
    wm_on(FakePlatform::single_monitor(), n, no_gaps())
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) {
    wm.handle(Event::Command(c.into()));
}

fn rect(wm: &Wm<FakePlatform>, id: isize) -> Rect {
    wm.platform().rect_of(W(id)).unwrap()
}

fn visible(wm: &Wm<FakePlatform>) -> Vec<isize> {
    wm.managed().map(|h| h.0).collect()
}

fn hidden(wm: &Wm<FakePlatform>, id: isize) -> bool {
    wm.platform().is_hidden(W(id))
}

// ---- stacks -----------------------------------------------------------------

#[test]
fn stack_puts_two_windows_in_one_tile() {
    let mut wm = wm_with(3);
    // 1 is on the left, 2 top-right, 3 bottom-right. Stack 1 onto 2.
    cmd(&mut wm, "stack right");
    assert_eq!(visible(&wm), vec![1, 3]);
    assert!(hidden(&wm, 2));
    assert_eq!(wm.focused(), Some(W(1)));
    // 1 now holds 2's old slot (the top of the right column): the layout
    // has two tiles, 1 and 3. The stack's tab bar takes the top 28px.
    assert_eq!(rect(&wm, 1), Rect::new(0, 28, 960, 1012));
    assert_eq!(rect(&wm, 3), Rect::new(960, 0, 960, 1040));
    let bars = &wm.platform().stackbars;
    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].rect, Rect::new(0, 0, 960, 28));
    assert_eq!(
        bars[0].tabs,
        vec![
            (W(2), "window 2".to_string()),
            (W(1), "window 1".to_string())
        ]
    );
    assert_eq!(bars[0].active, 1);
    assert!(bars[0].focused);

    cmd(&mut wm, "cycle-stack next");
    assert_eq!(wm.focused(), Some(W(2)));
    assert!(!hidden(&wm, 2) && hidden(&wm, 1));
    assert_eq!(rect(&wm, 2), Rect::new(0, 28, 960, 1012));
    assert_eq!(wm.platform().stackbars[0].active, 0);
    // The new window is shown and focused before the old one is hidden.
    let calls = &wm.platform().calls;
    let show = calls.iter().rposition(|c| *c == Call::Show(W(2))).unwrap();
    let focus = calls.iter().rposition(|c| *c == Call::Focus(W(2))).unwrap();
    let hide = calls.iter().rposition(|c| *c == Call::Hide(W(1))).unwrap();
    assert!(show < focus && focus < hide, "{calls:?}");

    let s = wm.snapshot();
    let ws = &s.monitors[0].workspaces[0];
    assert_eq!(ws.stacks, vec![vec![2, 1]]);
    assert_eq!(ws.windows.len(), 3);
    assert!(ws.windows.iter().any(|w| w.handle == 1 && w.stacked_hidden));
}

#[test]
fn a_late_hide_event_after_cycling_back_is_not_a_close() {
    let mut s = no_gaps();
    s.hide_method = HideMethod::Hide;
    let mut wm = wm_on(FakePlatform::single_monitor(), 2, s);
    cmd(&mut wm, "stack right");
    cmd(&mut wm, "cycle-stack next");
    // The event for hiding 2 (from `stack`) only arrives now, after 2 was
    // shown again; it must not make rustWM forget window 2.
    wm.handle(Event::WindowHidden(W(2)));
    wm.handle(Event::WindowHidden(W(1)));
    let s = wm.snapshot();
    assert_eq!(s.monitors[0].workspaces[0].stacks, vec![vec![2, 1]]);
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn closing_the_shown_stack_window_reveals_the_next() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "stack right");
    assert!(hidden(&wm, 2));
    wm.platform_mut().close_window(W(1));
    wm.handle(Event::WindowDestroyed(W(1)));
    assert!(!hidden(&wm, 2));
    assert_eq!(visible(&wm), vec![2]);
    // A stack of one is a plain tile again, without a tab bar.
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));
    assert!(wm.platform().stackbars.is_empty());
}

#[test]
fn unstack_and_stack_all() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "stack-all");
    assert_eq!(visible(&wm), vec![1]);
    assert!(hidden(&wm, 2) && hidden(&wm, 3));
    cmd(&mut wm, "unstack");
    assert_eq!(visible(&wm).len(), 2);
    cmd(&mut wm, "unstack-all");
    assert_eq!(visible(&wm).len(), 3);
    assert!(!hidden(&wm, 2) && !hidden(&wm, 3));
}

#[test]
fn stacked_windows_stay_hidden_across_workspace_switches() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "stack right");
    cmd(&mut wm, "workspace 2");
    assert!(hidden(&wm, 1) && hidden(&wm, 2));
    cmd(&mut wm, "workspace 1");
    assert!(!hidden(&wm, 1) && hidden(&wm, 2));
    // Quitting shows everything again.
    cmd(&mut wm, "quit");
    assert!(!hidden(&wm, 2));
}

// ---- focus history and workspaces ------------------------------------------

#[test]
fn focus_last_goes_back_and_forth() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "focus right");
    assert_eq!(wm.focused(), Some(W(2)));
    cmd(&mut wm, "focus last");
    assert_eq!(wm.focused(), Some(W(1)));
    cmd(&mut wm, "focus last");
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn focus_last_switches_to_the_workspace_it_is_on() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "move-to-workspace 3");
    assert_eq!(wm.focused(), Some(W(1)));
    cmd(&mut wm, "focus last");
    assert_eq!(wm.focused(), Some(W(2)));
    assert_eq!(wm.snapshot().monitors[0].active_workspace, "3");
}

#[test]
fn workspace_keywords_and_back_and_forth() {
    let mut s = no_gaps();
    s.workspace_back_and_forth = true;
    let mut wm = wm_on(FakePlatform::single_monitor(), 1, s);
    let active = |wm: &Wm<FakePlatform>| wm.snapshot().monitors[0].active_workspace.clone();
    cmd(&mut wm, "workspace 4");
    cmd(&mut wm, "workspace last");
    assert_eq!(active(&wm), "1");
    cmd(&mut wm, "workspace 1");
    assert_eq!(active(&wm), "4", "back and forth");
    cmd(&mut wm, "workspace next");
    assert_eq!(active(&wm), "5");
    cmd(&mut wm, "workspace prev");
    assert_eq!(active(&wm), "4");
    cmd(&mut wm, "workspace 9");
    cmd(&mut wm, "workspace next");
    assert_eq!(active(&wm), "1", "wraps");
    cmd(&mut wm, "workspace empty");
    assert_eq!(active(&wm), "2");
}

#[test]
fn move_to_workspace_follow() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "move-to-workspace 3 follow");
    assert_eq!(wm.snapshot().monitors[0].active_workspace, "3");
    assert_eq!(wm.focused(), Some(W(1)));
    assert!(!hidden(&wm, 1) && hidden(&wm, 2));
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

#[test]
fn swap_workspaces_between_monitors() {
    let mut p = FakePlatform::dual_monitor();
    p.add_window(1, "a.exe", "a");
    p.add_window(2, "b.exe", "b");
    p.place(W(2), Rect::new(2000, 100, 800, 600));
    p.foreground = Some(W(1));
    let mut wm = Wm::with_settings(p, no_gaps());
    wm.start().unwrap();
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
    cmd(&mut wm, "swap-workspaces right");
    assert_eq!(rect(&wm, 1), Rect::new(1920, 0, 1920, 1080));
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));
    assert_eq!(wm.focused_monitor(), 1);
    assert_eq!(wm.focused(), Some(W(1)));
}

// ---- layouts ----------------------------------------------------------------

#[test]
fn new_layouts_and_flip() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "layout columns");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 640, 1040));
    assert_eq!(rect(&wm, 3), Rect::new(1280, 0, 640, 1040));
    cmd(&mut wm, "flip-layout horizontal");
    assert_eq!(rect(&wm, 1), Rect::new(1280, 0, 640, 1040));
    cmd(&mut wm, "flip-layout horizontal");
    cmd(&mut wm, "layout rows");
    assert_eq!(rect(&wm, 2).h, 346);
    cmd(&mut wm, "layout center");
    assert_eq!(rect(&wm, 1), Rect::new(480, 0, 960, 1040));
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 480, 1040));
    cmd(&mut wm, "layout grid");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 520));
}

#[test]
fn cycle_layout_uses_the_configured_cycle() {
    let mut s = no_gaps();
    s.layout_cycle = vec![LayoutKind::Dwindle, LayoutKind::Grid, LayoutKind::Center];
    let mut wm = wm_on(FakePlatform::single_monitor(), 2, s);
    let layout = |wm: &Wm<FakePlatform>| wm.snapshot().monitors[0].workspaces[0].layout;
    cmd(&mut wm, "cycle-layout");
    assert_eq!(layout(&wm), LayoutKind::Grid);
    cmd(&mut wm, "cycle-layout prev");
    cmd(&mut wm, "cycle-layout prev");
    assert_eq!(layout(&wm), LayoutKind::Center);
}

#[test]
fn toggle_split_turns_a_dwindle_split() {
    let mut wm = wm_with(2);
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
    cmd(&mut wm, "toggle-split");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 520));
    assert_eq!(rect(&wm, 2), Rect::new(0, 520, 1920, 520));
    cmd(&mut wm, "toggle-split");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
}

#[test]
fn workspace_overrides_set_layout_and_gaps() {
    let mut s = no_gaps();
    s.workspace_overrides.insert(
        "1".into(),
        WorkspaceOverride {
            layout: Some(LayoutKind::Columns),
            gaps: Some(Gaps {
                inner: 10,
                outer: 10,
                smart: false,
            }),
        },
    );
    let wm = wm_on(FakePlatform::single_monitor(), 2, s);
    assert_eq!(rect(&wm, 1), Rect::new(10, 10, 945, 1020));
    assert_eq!(rect(&wm, 2), Rect::new(965, 10, 945, 1020));
}

// ---- maximize, pin, floating --------------------------------------------------

#[test]
fn toggle_maximize_fills_the_work_area() {
    let mut s = Settings::default();
    s.gaps.smart = false;
    let mut wm = wm_on(FakePlatform::single_monitor(), 2, s);
    cmd(&mut wm, "toggle-maximize");
    assert_eq!(rect(&wm, 1), Rect::new(8, 8, 1904, 1024));
    // The other window takes the rest of the layout underneath.
    assert_eq!(rect(&wm, 2), Rect::new(8, 8, 1904, 1024));
    assert!(wm.snapshot().focused_window.unwrap().maximized);
    cmd(&mut wm, "toggle-maximize");
    assert_eq!(rect(&wm, 1).w, 948);
}

#[test]
fn pinned_windows_follow_workspace_switches() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "toggle-pin");
    assert!(wm.snapshot().focused_window.unwrap().pinned);
    cmd(&mut wm, "workspace 2");
    assert!(!hidden(&wm, 1), "pinned window stays on screen");
    assert!(hidden(&wm, 2));
    assert_eq!(visible(&wm), vec![1]);
    cmd(&mut wm, "workspace 1");
    assert_eq!(visible(&wm), vec![2, 1]);
}

#[test]
fn pin_rule_and_monitor_rule() {
    let mut p = FakePlatform::dual_monitor();
    p.foreground = None;
    let mut s = no_gaps();
    s.rules = vec![
        Rule::new(Some("mpv.exe"), None, None, vec![RuleAction::Pin]).unwrap(),
        Rule::new(Some("chat.exe"), None, None, vec![RuleAction::Monitor(2)]).unwrap(),
    ];
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    let mpv = wm.platform_mut().add_window(1, "mpv.exe", "video");
    wm.handle(Event::WindowShown(mpv));
    let chat = wm.platform_mut().add_window(2, "chat.exe", "chat");
    wm.handle(Event::WindowShown(chat));
    let state = wm.snapshot();
    let w1 = &state.monitors[0].workspaces[0].windows[0];
    assert!(w1.pinned && w1.floating);
    assert_eq!(state.monitors[1].workspaces[0].windows[0].handle, 2);
}

#[test]
fn floating_windows_move_and_resize_by_keyboard() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "toggle-float");
    let r = rect(&wm, 1);
    cmd(&mut wm, "move right");
    assert_eq!(rect(&wm, 1), Rect::new(r.x + 50, r.y, r.w, r.h));
    cmd(&mut wm, "resize down 30");
    assert_eq!(rect(&wm, 1).h, r.h + 30);
    cmd(&mut wm, "resize left");
    assert_eq!(rect(&wm, 1), Rect::new(r.x, r.y, r.w + 50, r.h + 30));
    wm.platform_mut().place(W(1), Rect::new(0, 0, 400, 300));
    cmd(&mut wm, "center");
    assert_eq!(rect(&wm, 1), Rect::new(760, 370, 400, 300));
}

#[test]
fn resize_edge_moves_the_divider() {
    let mut wm = wm_with(2);
    // 1 is left: its right edge has a neighbour, so it moves right.
    cmd(&mut wm, "resize right 100");
    assert!((rect(&wm, 1).w - 1060).abs() <= 5, "{:?}", rect(&wm, 1));
    // From the right window, `resize right` moves its left edge right.
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "resize right 100");
    assert!((rect(&wm, 2).x - 1160).abs() <= 5, "{:?}", rect(&wm, 2));
    cmd(&mut wm, "resize left 200");
    assert!((rect(&wm, 2).x - 960).abs() <= 5, "{:?}", rect(&wm, 2));
}

#[test]
fn resize_edge_respects_flip() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "flip-layout horizontal");
    assert_eq!(rect(&wm, 1), Rect::new(960, 0, 960, 1040));
    cmd(&mut wm, "resize left 100");
    assert!((rect(&wm, 1).x - 860).abs() <= 5, "{:?}", rect(&wm, 1));
}

// ---- pause, manage, unmanage ---------------------------------------------------

#[test]
fn pause_leaves_new_windows_alone_until_resumed() {
    let mut wm = wm_with(1);
    cmd(&mut wm, "toggle-pause");
    assert!(wm.snapshot().paused);
    let h = wm.platform_mut().add_window(5, "new.exe", "new");
    wm.handle(Event::WindowShown(h));
    assert_eq!(visible(&wm), vec![1]);
    assert_eq!(rect(&wm, 5), Rect::new(100, 100, 800, 600));
    cmd(&mut wm, "toggle-pause");
    assert_eq!(visible(&wm), vec![1, 5]);
    assert_eq!(rect(&wm, 5), Rect::new(960, 0, 960, 1040));
}

#[test]
fn unmanage_and_manage() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "unmanage");
    assert_eq!(visible(&wm), vec![2]);
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));
    // It is not picked up again by its own events.
    wm.handle(Event::WindowShown(W(1)));
    assert_eq!(visible(&wm), vec![2]);

    let tool = wm.platform_mut().add_window(7, "tool.exe", "tool");
    wm.platform_mut().windows.get_mut(&tool).unwrap().manageable = false;
    wm.platform_mut().foreground = Some(tool);
    cmd(&mut wm, "manage");
    assert!(visible(&wm).contains(&7));
}

// ---- mouse and opacity -------------------------------------------------------

#[test]
fn mouse_follows_keyboard_focus() {
    let mut s = no_gaps();
    s.mouse_follows_focus = true;
    let mut wm = wm_on(FakePlatform::single_monitor(), 2, s);
    wm.platform_mut().cursor = Some((10, 10));
    cmd(&mut wm, "focus right");
    assert_eq!(wm.platform().cursor, Some((1440, 520)));
    // Already inside the newly focused window: left alone.
    wm.platform_mut().cursor = Some((100, 100));
    cmd(&mut wm, "focus left");
    assert_eq!(wm.platform().cursor, Some((100, 100)));
}

#[test]
fn inactive_windows_turn_translucent() {
    let mut s = no_gaps();
    s.inactive_opacity = 0.8;
    let mut wm = wm_on(FakePlatform::single_monitor(), 3, s);
    let op = |wm: &Wm<FakePlatform>, id| wm.platform().opacity.get(&W(id)).copied();
    assert_eq!(op(&wm, 1), None);
    assert_eq!(op(&wm, 2), Some(204));
    cmd(&mut wm, "focus right");
    assert_eq!(op(&wm, 1), Some(204));
    assert_eq!(op(&wm, 2), None);
    cmd(&mut wm, "quit");
    assert!(wm.platform().opacity.is_empty());
}

#[test]
fn exec_on_start_runs_once() {
    let mut s = no_gaps();
    s.exec_on_start = vec!["yasb.exe".into()];
    let wm = wm_on(FakePlatform::single_monitor(), 0, s);
    assert!(
        wm.platform()
            .calls
            .contains(&Call::Spawn("yasb.exe".into()))
    );
}

// ---- off-screen hiding --------------------------------------------------------

#[test]
fn settings_reach_the_platform() {
    let mut s = no_gaps();
    s.animation_ms = 150;
    let wm = wm_on(FakePlatform::single_monitor(), 0, s);
    let o = wm.platform().options.unwrap();
    assert_eq!(
        (o.hide_method, o.animation_ms),
        (HideMethod::Offscreen, 150)
    );
}

#[test]
fn clicking_a_hidden_window_on_the_taskbar_switches_to_it() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "move-to-workspace 3");
    assert!(hidden(&wm, 2));
    // The user clicks window 2's taskbar button: Windows activates it.
    wm.platform_mut().foreground = Some(W(2));
    wm.handle(Event::WindowFocused(W(2)));
    assert_eq!(wm.snapshot().monitors[0].active_workspace, "3");
    assert!(!hidden(&wm, 2) && hidden(&wm, 1));
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn a_stale_focus_event_does_not_switch_back() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "move-to-workspace 3");
    // A focus event for 2 from before it moved arrives late; 1 has the
    // foreground by now.
    assert_eq!(wm.platform().foreground, Some(W(1)));
    wm.handle(Event::WindowFocused(W(2)));
    assert_eq!(wm.snapshot().monitors[0].active_workspace, "1");
}

#[test]
fn activating_a_hidden_stack_window_brings_it_to_the_front() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "stack right");
    assert!(hidden(&wm, 2));
    wm.platform_mut().foreground = Some(W(2));
    wm.handle(Event::WindowFocused(W(2)));
    assert!(!hidden(&wm, 2) && hidden(&wm, 1));
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn display_changes_hide_off_screen_windows_again() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "focus right");
    cmd(&mut wm, "move-to-workspace 3");
    let hides = |wm: &Wm<FakePlatform>| {
        wm.platform()
            .calls
            .iter()
            .filter(|c| **c == Call::Hide(W(2)))
            .count()
    };
    assert_eq!(hides(&wm), 1);
    wm.handle(Event::DisplayChanged);
    assert_eq!(hides(&wm), 2);
}

// ---- master count and side --------------------------------------------------

#[test]
fn master_count_and_side_commands() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "layout master");
    cmd(&mut wm, "master-count +1");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 520));
    assert_eq!(rect(&wm, 2), Rect::new(0, 520, 960, 520));
    assert_eq!(rect(&wm, 3), Rect::new(960, 0, 960, 1040));
    cmd(&mut wm, "master-count 1");
    cmd(&mut wm, "master-side right");
    assert_eq!(rect(&wm, 1), Rect::new(960, 0, 960, 1040));
    cmd(&mut wm, "master-side next");
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 520));
    // Never below one master.
    cmd(&mut wm, "master-count -5");
    assert_eq!(rect(&wm, 2), Rect::new(0, 520, 960, 520));
}

#[test]
fn master_settings_from_the_config() {
    let mut s = no_gaps();
    s.default_layout = LayoutKind::Master;
    s.master_side = Side::Bottom;
    let wm = wm_on(FakePlatform::single_monitor(), 2, s);
    assert_eq!(rect(&wm, 1), Rect::new(0, 520, 1920, 520));
}

// ---- focus-window and minimize ----------------------------------------------

#[test]
fn focus_window_switches_workspace_and_stack_tab() {
    let mut wm = wm_with(3);
    cmd(&mut wm, "focus-window 3");
    cmd(&mut wm, "move-to-workspace 2");
    assert!(hidden(&wm, 3));
    cmd(&mut wm, "focus-window title:window 3");
    assert_eq!(wm.snapshot().monitors[0].active_workspace, "2");
    assert_eq!(wm.focused(), Some(W(3)));
    assert!(!hidden(&wm, 3));

    cmd(&mut wm, "workspace 1");
    cmd(&mut wm, "focus-window 1");
    cmd(&mut wm, "stack right");
    assert!(hidden(&wm, 2));
    cmd(&mut wm, "focus-window 2");
    assert!(!hidden(&wm, 2) && hidden(&wm, 1));
    assert_eq!(wm.focused(), Some(W(2)));
}

#[test]
fn focus_window_by_exe_cycles_through_matches() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(10, "firefox.exe", "a");
    p.add_window(11, "firefox.exe", "b");
    let mut wm = wm_on(p, 1, no_gaps());
    cmd(&mut wm, "focus-window exe:firefox\\.exe");
    let first = wm.focused().unwrap();
    assert!(first == W(10) || first == W(11));
    cmd(&mut wm, "focus-window exe:FIREFOX\\.exe");
    let second = wm.focused().unwrap();
    assert!(second != first && (second == W(10) || second == W(11)));
}

#[test]
fn minimize_unmanages_and_focuses_the_next_window() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "minimize");
    assert!(wm.platform().calls.contains(&Call::Minimize(W(1))));
    assert_eq!(visible(&wm), vec![2]);
    assert_eq!(wm.focused(), Some(W(2)));
    assert_eq!(rect(&wm, 2), Rect::new(0, 0, 1920, 1040));
}

// ---- rules, work area -------------------------------------------------------

#[test]
fn size_rule_floats_at_that_size() {
    let mut s = no_gaps();
    s.rules = vec![
        Rule::new(
            Some("calc.exe"),
            None,
            None,
            vec![RuleAction::Size(600, 400)],
        )
        .unwrap(),
    ];
    let mut wm = wm_with(1);
    wm.set_settings(s);
    wm.platform_mut().add_window(5, "calc.exe", "Calculator");
    wm.handle(Event::WindowShown(W(5)));
    assert_eq!(rect(&wm, 5), Rect::new(660, 320, 600, 400));
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

#[test]
fn work_area_offset_leaves_room_for_a_bar() {
    let mut s = no_gaps();
    s.work_area_offset = Offsets {
        top: 30,
        ..Offsets::default()
    };
    let mut wm = wm_on(FakePlatform::single_monitor(), 1, s);
    assert_eq!(rect(&wm, 1), Rect::new(0, 30, 1920, 1010));
    wm.set_settings(no_gaps());
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

// ---- swallowing -------------------------------------------------------------

fn swallowing() -> Settings {
    let mut s = no_gaps();
    s.swallow.enabled = true;
    s.swallow.terminals = Some(regex::Regex::new("(?i)^(?:wezterm-gui\\.exe)$").unwrap());
    s
}

#[test]
fn an_app_started_from_a_terminal_swallows_it() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "wezterm-gui.exe", "shell");
    p.add_window(2, "notes.exe", "notes");
    p.foreground = Some(W(1));
    p.pids = [(W(1), 100), (W(2), 200)].into();
    let mut wm = Wm::with_settings(p, swallowing());
    wm.start().unwrap();
    // mpv (pid 300) is started by the shell (pid 150) inside the terminal.
    {
        let p = wm.platform_mut();
        p.add_window(3, "mpv.exe", "video");
        p.pids.insert(W(3), 300);
        p.parents = [(300, 150), (150, 100)].into();
    }
    wm.handle(Event::WindowShown(W(3)));
    assert!(hidden(&wm, 1));
    assert_eq!(wm.focused(), Some(W(3)));
    // mpv took the terminal's tile; notes keeps its own.
    assert_eq!(rect(&wm, 2), Rect::new(960, 0, 960, 1040));
    assert_eq!(rect(&wm, 3).x, 0);

    wm.platform_mut().close_window(W(3));
    wm.handle(Event::WindowDestroyed(W(3)));
    assert!(!hidden(&wm, 1));
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 960, 1040));
}

#[test]
fn unrelated_windows_are_not_swallowed() {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "wezterm-gui.exe", "shell");
    p.foreground = Some(W(1));
    p.pids = [(W(1), 100)].into();
    let mut wm = Wm::with_settings(p, swallowing());
    wm.start().unwrap();
    {
        let p = wm.platform_mut();
        p.add_window(3, "mpv.exe", "video");
        p.pids.insert(W(3), 300);
        p.parents = [(300, 4)].into();
    }
    wm.handle(Event::WindowShown(W(3)));
    assert_eq!(visible(&wm), vec![1, 3]);
}

#[test]
fn stack_tab_titles_follow_title_changes() {
    let mut wm = wm_with(2);
    cmd(&mut wm, "stack right");
    if let Some(w) = wm.platform_mut().windows.get_mut(&W(2)) {
        w.title = "renamed".into();
    }
    wm.handle(Event::WindowTitleChanged(W(2)));
    assert_eq!(wm.platform().stackbars[0].tabs[0].1, "renamed");
}
