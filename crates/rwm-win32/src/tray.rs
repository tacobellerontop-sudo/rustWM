//! The notification-area (tray) icon: a menu to reload, retile or quit, and
//! balloon notifications such as config errors.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use rwm_core::Event;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, IDI_APPLICATION, LoadIconW,
    MF_SEPARATOR, MF_STRING, RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU,
    WM_LBUTTONUP, WM_RBUTTONUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::events::send;

/// Message the icon sends to our hidden window.
pub(crate) const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

/// The hidden window that owns the icon, for notifications from other threads.
static OWNER: AtomicIsize = AtomicIsize::new(0);
/// Explorer broadcasts this after it restarts; the icon must be re-added.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Opened by the "Open config" menu item.
static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

const MENU: &[(usize, &str)] = &[
    (1, "Open config"),
    (2, "Reload config"),
    (3, "Retile"),
    (0, ""),
    (4, "Quit rustWM"),
];

/// Sets the file the tray's "Open config" item opens.
pub fn set_config_path(path: PathBuf) {
    let _ = CONFIG_PATH.set(path);
}

fn copy_into(dst: &mut [u16], text: &str) {
    let len = dst.len() - 1;
    for (d, s) in dst.iter_mut().zip(text.encode_utf16().take(len)) {
        *d = s;
    }
}

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

pub(crate) fn add(hwnd: HWND) {
    // SAFETY: plain Win32 calls with a fully initialised NOTIFYICONDATAW.
    unsafe {
        if TASKBAR_CREATED.load(Ordering::Relaxed) == 0 {
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(w!("TaskbarCreated")),
                Ordering::Relaxed,
            );
        }
        let mut nid = base(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        copy_into(&mut nid.szTip, "rustWM");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
    OWNER.store(hwnd.0 as isize, Ordering::Relaxed);
}

pub(crate) fn remove(hwnd: HWND) {
    // SAFETY: removing the icon we added for this window.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base(hwnd));
    }
    OWNER.store(0, Ordering::Relaxed);
}

/// Whether `msg` is Explorer announcing a fresh taskbar.
pub(crate) fn is_taskbar_created(msg: u32) -> bool {
    let tc = TASKBAR_CREATED.load(Ordering::Relaxed);
    tc != 0 && msg == tc
}

/// Handles a `WM_TRAY` message; `event` is the mouse message in `lParam`.
pub(crate) fn on_message(hwnd: HWND, event: u32) {
    if !matches!(event, WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU) {
        return;
    }
    // SAFETY: standard popup-menu handling on our own window. The window must
    // be foreground or the menu won't close when clicking elsewhere.
    let choice = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for (id, label) in MENU {
            if *id == 0 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            } else {
                let _ = AppendMenuW(menu, MF_STRING, *id, &HSTRING::from(*label));
            }
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        choice.0 as usize
    };
    match choice {
        1 => {
            if let Some(path) = CONFIG_PATH.get() {
                // SAFETY: opening a file path with its default handler.
                unsafe {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        &HSTRING::from(path.as_os_str()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
            }
        }
        2 => send(Event::Command("reload".into())),
        3 => send(Event::Command("retile".into())),
        4 => send(Event::Command("quit".into())),
        _ => {}
    }
}

/// Shows a balloon notification from the tray icon. Safe from any thread;
/// does nothing if the icon is not there.
pub fn notify(title: &str, text: &str) {
    let owner = OWNER.load(Ordering::Relaxed);
    if owner == 0 {
        return;
    }
    let mut nid = base(HWND(owner as _));
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_WARNING;
    copy_into(&mut nid.szInfoTitle, title);
    copy_into(&mut nid.szInfo, text);
    // SAFETY: modifying our own icon with a fully initialised struct.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}
