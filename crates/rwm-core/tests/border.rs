//! The focus border and focus-follows-mouse.

use rwm_core::fake::FakePlatform;
use rwm_core::wm::Settings;
use rwm_core::{Border, Event, Rect, WindowHandle, Wm};

const W: fn(isize) -> WindowHandle = WindowHandle;

fn settings() -> Settings {
    let mut s = Settings::default();
    s.gaps.inner = 0;
    s.gaps.outer = 0;
    s
}

fn start(s: Settings) -> Wm<FakePlatform> {
    let mut p = FakePlatform::single_monitor();
    p.add_window(1, "a.exe", "one");
    p.add_window(2, "b.exe", "two");
    p.foreground = Some(W(1));
    let mut wm = Wm::with_settings(p, s);
    wm.start().unwrap();
    wm
}

fn cmd(wm: &mut Wm<FakePlatform>, c: &str) {
    wm.handle(Event::Command(c.into()));
}

fn border(wm: &Wm<FakePlatform>) -> Option<Border> {
    wm.platform().border
}

/// Something unmanaged takes the foreground, like the Start menu.
fn focus_unmanaged(wm: &mut Wm<FakePlatform>, class: &str) {
    let p = wm.platform_mut();
    p.add_window(99, "shell.exe", "");
    let w = p.windows.get_mut(&W(99)).unwrap();
    w.manageable = false;
    w.class = class.into();
    p.foreground = Some(W(99));
    wm.handle(Event::WindowFocused(W(99)));
}

#[test]
fn border_follows_focus() {
    let mut wm = start(settings());
    assert_eq!(
        border(&wm),
        Some(Border {
            window: W(1),
            rect: Rect::new(0, 0, 960, 1040),
            width: 3,
            color: 0x89b4fa,
        })
    );
    cmd(&mut wm, "focus right");
    assert_eq!(border(&wm).unwrap().window, W(2));
    assert_eq!(border(&wm).unwrap().rect, Rect::new(960, 0, 960, 1040));
    // Layout changes move it along.
    cmd(&mut wm, "resize grow");
    assert_eq!(
        border(&wm).unwrap().rect,
        wm.platform().rect_of(W(2)).unwrap()
    );
}

#[test]
fn border_hides_when_something_else_has_focus() {
    let mut wm = start(settings());
    focus_unmanaged(&mut wm, "Windows.UI.Core.CoreWindow");
    assert_eq!(border(&wm), None);
    wm.platform_mut().foreground = Some(W(2));
    wm.handle(Event::WindowFocused(W(2)));
    assert_eq!(border(&wm).unwrap().window, W(2));
}

#[test]
fn border_hides_while_dragging_and_in_fullscreen() {
    let mut wm = start(settings());
    wm.handle(Event::WindowMoveStarted(W(1)));
    assert_eq!(border(&wm), None);
    wm.handle(Event::WindowMoveEnded(W(1)));
    assert!(border(&wm).is_some());
    cmd(&mut wm, "toggle-fullscreen");
    assert_eq!(border(&wm), None);
    cmd(&mut wm, "toggle-fullscreen");
    assert!(border(&wm).is_some());
    cmd(&mut wm, "quit");
    assert_eq!(border(&wm), None);
}

#[test]
fn border_can_be_turned_off_and_scales() {
    let mut s = settings();
    s.border.enabled = false;
    let wm = start(s);
    assert_eq!(border(&wm), None);

    let mut p = FakePlatform::single_monitor();
    p.monitors[0].scale = 1.5;
    p.add_window(1, "a.exe", "one");
    p.foreground = Some(W(1));
    let mut wm = Wm::with_settings(p, settings());
    wm.start().unwrap();
    assert_eq!(border(&wm).unwrap().width, 5);
}

#[test]
fn focus_follows_mouse_only_when_enabled() {
    let mut wm = start(settings());
    wm.handle(Event::MouseOver(W(2)));
    assert_eq!(wm.focused(), Some(W(1)));

    let mut s = settings();
    s.focus_follows_mouse = true;
    let mut wm = start(s);
    wm.handle(Event::MouseOver(W(2)));
    assert_eq!(wm.focused(), Some(W(2)));
    assert_eq!(wm.platform().foreground, Some(W(2)));
    assert_eq!(border(&wm).unwrap().window, W(2));
}

#[test]
fn focus_follows_mouse_leaves_menus_alone() {
    let mut s = settings();
    s.focus_follows_mouse = true;
    let mut wm = start(s);
    // The Start menu is open: don't steal focus from it.
    focus_unmanaged(&mut wm, "Windows.UI.Core.CoreWindow");
    wm.handle(Event::MouseOver(W(2)));
    assert_eq!(wm.platform().foreground, Some(W(99)));
    // But coming back from the desktop is fine.
    wm.platform_mut().windows.get_mut(&W(99)).unwrap().class = "Progman".into();
    wm.handle(Event::MouseOver(W(2)));
    assert_eq!(wm.platform().foreground, Some(W(2)));
    // And not while dragging.
    wm.handle(Event::WindowMoveStarted(W(2)));
    wm.handle(Event::MouseOver(W(1)));
    assert_eq!(wm.platform().foreground, Some(W(2)));
}
