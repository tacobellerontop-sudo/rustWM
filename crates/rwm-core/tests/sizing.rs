//! Fixed-size windows, `tile` rules and resizing tiles with the mouse.

use rwm_core::fake::FakePlatform;
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

fn start(p: FakePlatform, s: Settings) -> Wm<FakePlatform> {
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    wm
}

fn two_windows() -> FakePlatform {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "app.exe", "one");
    p.add_window(2, "app.exe", "two");
    p.foreground = Some(W(1));
    p
}

fn rect(wm: &Wm<FakePlatform>, id: isize) -> Rect {
    wm.platform().rect_of(W(id)).unwrap()
}

fn floating(wm: &Wm<FakePlatform>, id: isize) -> bool {
    wm.snapshot()
        .monitors
        .iter()
        .flat_map(|m| &m.workspaces)
        .flat_map(|w| &w.windows)
        .find(|w| w.handle == id)
        .unwrap()
        .floating
}

#[test]
fn window_without_sizing_border_floats() {
    let mut p = two_windows();
    p.windows.get_mut(&W(2)).unwrap().resizable = false;
    let wm = start(p, settings());
    assert!(floating(&wm, 2));
    // Window 1 gets the whole screen.
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
}

#[test]
fn tile_rule_overrides_detection() {
    let mut p = two_windows();
    p.windows.get_mut(&W(2)).unwrap().resizable = false;
    let mut s = settings();
    s.rules = vec![Rule::new(None, None, Some("two"), vec![RuleAction::Tile]).unwrap()];
    let mut wm = start(p, s);
    assert!(!floating(&wm, 2));
    // Even when it refuses its size, it stays tiled.
    wm.platform_mut().windows.get_mut(&W(2)).unwrap().rect = Rect::new(960, 0, 1400, 1040);
    wm.handle(Event::Verify);
    assert!(!floating(&wm, 2));
}

#[test]
fn window_that_refuses_its_slot_is_floated() {
    let mut p = two_windows();
    // Like the Riot Client: has a border but enforces a minimum size.
    p.min_size.insert(W(2), (1200, 800));
    let mut wm = start(p, settings());
    assert!(wm.take_verify_request());
    assert!(!wm.take_verify_request());
    wm.handle(Event::Verify);
    assert!(floating(&wm, 2));
    assert_eq!(rect(&wm, 1), Rect::new(0, 0, 1920, 1040));
    // Keeps its own size, centred.
    assert_eq!(rect(&wm, 2), Rect::new(360, 0, 1200, 1040));
}

#[test]
fn tiling_a_floated_window_by_hand_sticks() {
    let mut p = two_windows();
    p.min_size.insert(W(2), (1200, 800));
    p.foreground = Some(W(2));
    let mut wm = start(p, settings());
    wm.handle(Event::Verify);
    assert!(floating(&wm, 2));
    wm.handle(Event::Command("toggle-float".into()));
    wm.handle(Event::Verify);
    assert!(!floating(&wm, 2));
}

#[test]
fn dragging_an_edge_resizes_the_tile() {
    let mut wm = start(two_windows(), settings());
    // The user drags window 1's right edge from 960 to 1200.
    wm.platform_mut().windows.get_mut(&W(1)).unwrap().rect = Rect::new(0, 0, 1200, 1040);
    wm.handle(Event::WindowMoveEnded(W(1)));
    let (a, b) = (rect(&wm, 1), rect(&wm, 2));
    assert!((a.w - 1200).abs() <= 5, "{a:?}");
    assert_eq!(b.x, a.right());
    assert_eq!(b.right(), 1920);
    assert!(!floating(&wm, 1));
}
