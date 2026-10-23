//! Global hotkeys through a low-level keyboard hook.
//!
//! `RegisterHotKey` cannot claim most Win+key combinations (Windows reserves
//! them), so rustWM watches the keyboard with `WH_KEYBOARD_LL` instead. When a
//! chord matches a binding, the key is swallowed and its command is sent to
//! the core. A lone Win tap still opens Start: we only suppress Start after a
//! chord actually fired.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

use rwm_config::Chord;
use rwm_core::Event;
use tracing::warn;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::events::send;

static BINDINGS: RwLock<Option<HashMap<Chord, String>>> = RwLock::new(None);

/// A Win+key chord fired since Win went down, so Start must not open on release.
static WIN_CHORD_FIRED: AtomicBool = AtomicBool::new(false);

/// Keys whose key-down we swallowed; their key-up is swallowed too.
static SWALLOWED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// An unassigned virtual key, injected to stop the Start menu opening.
const VK_NONE: u16 = 0xE8;

/// Replaces the active key bindings. Safe to call from any thread at any time.
pub fn set_bindings(bindings: impl IntoIterator<Item = (Chord, String)>) {
    let map = bindings.into_iter().collect();
    if let Ok(mut b) = BINDINGS.write() {
        *b = Some(map);
    }
}

fn pressed(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: no preconditions.
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

fn is_modifier(vk: u32) -> bool {
    [
        VK_LWIN,
        VK_RWIN,
        VK_SHIFT,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
    ]
    .iter()
    .any(|m| m.0 as u32 == vk)
}

fn lookup(chord: &Chord) -> Option<String> {
    BINDINGS.read().ok()?.as_ref()?.get(chord).cloned()
}

/// Taps an unassigned key so Windows sees Win+<something> and skips Start.
fn suppress_start_menu() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(VK_NONE),
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    // SAFETY: a valid slice of fully initialised keyboard inputs.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Decides whether to swallow a key event. Must return quickly: Windows drops
/// hooks that take longer than `LowLevelHooksTimeout`.
fn on_key(msg: u32, vk: u32) -> bool {
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    if is_modifier(vk) {
        let win = vk == VK_LWIN.0 as u32 || vk == VK_RWIN.0 as u32;
        if up && win && WIN_CHORD_FIRED.swap(false, Ordering::Relaxed) {
            suppress_start_menu();
        }
        return false;
    }
    if down {
        let chord = Chord {
            win: pressed(VK_LWIN) || pressed(VK_RWIN),
            ctrl: pressed(VK_CONTROL),
            alt: pressed(VK_MENU),
            shift: pressed(VK_SHIFT),
            vk: vk as u16,
        };
        if let Some(cmd) = lookup(&chord) {
            send(Event::Command(cmd));
            if chord.win {
                WIN_CHORD_FIRED.store(true, Ordering::Relaxed);
            }
            if let Ok(mut s) = SWALLOWED.lock() {
                if !s.contains(&vk) {
                    s.push(vk);
                }
            }
            return true;
        }
        return false;
    }
    if up {
        if let Ok(mut s) = SWALLOWED.lock() {
            if let Some(i) = s.iter().position(|k| *k == vk) {
                s.swap_remove(i);
                return true;
            }
        }
    }
    false
}

unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore injected input, including our own Start-menu suppression.
        if !kb.flags.contains(LLKHF_INJECTED) && on_key(wparam.0 as u32, kb.vkCode) {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the hook arguments on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Installs the hook on the calling thread, which must pump messages.
pub(crate) fn install() -> Option<HHOOK> {
    // SAFETY: valid hook procedure and module handle.
    let hook = unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(ll_keyboard_proc),
            Some(instance.into()),
            0,
        )
    };
    match hook {
        Ok(h) => Some(h),
        Err(e) => {
            warn!(error = %e, "could not install keyboard hook; hotkeys disabled");
            None
        }
    }
}

pub(crate) fn uninstall(hook: HHOOK) {
    // SAFETY: `hook` came from `install` on this thread.
    let _ = unsafe { UnhookWindowsHookEx(hook) };
}
