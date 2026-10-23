//! The Win32 thread: WinEvent hooks and display-change notifications, turned
//! into [`Event`]s for the core thread.

use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use rwm_core::Event;
use tracing::{debug, warn};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{
    CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE,
    EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GA_ROOT,
    GetAncestor, GetMessageW, MSG, OBJID_WINDOW, PostThreadMessageW, RegisterClassW,
    SPI_SETWORKAREA, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_DISPLAYCHANGE, WM_QUIT, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{BOOL, w};

use crate::{border, tray, window};

/// Where every callback sends its events. Set once by [`EventThread::spawn`];
/// the callbacks are plain `extern "system"` functions with no user data.
static SENDER: OnceLock<Sender<Event>> = OnceLock::new();

pub(crate) fn send(event: Event) {
    if let Some(tx) = SENDER.get() {
        // The receiver is gone only during shutdown; nothing to do then.
        let _ = tx.send(event);
    }
}

/// Event ranges to hook. Kept narrow on purpose: hooking the whole range would
/// also deliver `EVENT_OBJECT_LOCATIONCHANGE`, which fires constantly.
const HOOK_RANGES: &[(u32, u32)] = &[
    (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
    (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
    (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
    (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
    (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE),
    (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
];

/// Maps a raw WinEvent to an [`Event`]. Kept tiny: it runs for every window
/// on the desktop and must never block.
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_invalid() || id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    // Destroyed windows can no longer be asked for their ancestor; the core
    // ignores destroys of windows it does not manage anyway.
    // SAFETY: GetAncestor accepts any handle.
    if event != EVENT_OBJECT_DESTROY && unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    let h = window::handle(hwnd);
    let ev = match event {
        EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => Event::WindowShown(h),
        EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED => Event::WindowHidden(h),
        EVENT_OBJECT_DESTROY => Event::WindowDestroyed(h),
        EVENT_SYSTEM_FOREGROUND => Event::WindowFocused(h),
        EVENT_SYSTEM_MINIMIZESTART => Event::WindowMinimized(h),
        EVENT_SYSTEM_MINIMIZEEND => Event::WindowRestored(h),
        EVENT_SYSTEM_MOVESIZESTART => Event::WindowMoveStarted(h),
        EVENT_SYSTEM_MOVESIZEEND => Event::WindowMoveEnded(h),
        EVENT_OBJECT_NAMECHANGE => Event::WindowTitleChanged(h),
        _ => return,
    };
    send(ev);
}

/// Window procedure of our hidden window. A top-level window (not a
/// message-only one) is needed because display changes are broadcast.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DISPLAYCHANGE => send(Event::DisplayChanged),
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => send(Event::DisplayChanged),
        tray::WM_TRAY => {
            tray::on_message(hwnd, (lparam.0 & 0xFFFF) as u32);
            return LRESULT(0);
        }
        m if tray::is_taskbar_created(m) => tray::add(hwnd),
        _ => {}
    }
    // SAFETY: forwarding the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Handle to the running Win32 event thread.
pub struct EventThread {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl EventThread {
    /// Starts the thread. Only one may exist per process.
    pub fn spawn(tx: Sender<Event>) -> std::io::Result<Self> {
        if SENDER.set(tx).is_err() {
            return Err(std::io::Error::other("event thread already started"));
        }
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new()
            .name("win32-events".into())
            .spawn(move || {
                // SAFETY: no preconditions.
                let _ = id_tx.send(unsafe { GetCurrentThreadId() });
                run();
            })?;
        let thread_id = id_rx
            .recv()
            .map_err(|_| std::io::Error::other("event thread died on startup"))?;
        Ok(Self {
            thread_id,
            join: Some(join),
        })
    }

    /// Stops the message loop and waits for the thread to exit.
    pub fn stop(mut self) {
        // SAFETY: posting WM_QUIT to a thread id we own.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn create_hidden_window() -> Option<HWND> {
    // SAFETY: standard class registration and window creation; the class name
    // and title are static wide strings.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("rustwm-events"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("rustwm-events"),
            w!("rustWM"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

fn run() {
    let hooks: Vec<HWINEVENTHOOK> = HOOK_RANGES
        .iter()
        .filter_map(|&(min, max)| {
            // SAFETY: out-of-context hook with a valid callback; events are
            // delivered on this thread while it pumps messages.
            let hook = unsafe {
                SetWinEventHook(
                    min,
                    max,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if hook.is_invalid() {
                warn!(min, max, "SetWinEventHook failed");
                None
            } else {
                Some(hook)
            }
        })
        .collect();
    let hidden = create_hidden_window();
    border::create();
    match hidden {
        Some(h) => tray::add(h),
        None => warn!("could not create the hidden window; display changes will be missed"),
    }
    debug!(hooks = hooks.len(), "win32 event loop running");

    let mut msg = MSG::default();
    // SAFETY: standard message pump; GetMessageW returns 0 on WM_QUIT, -1 on error.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for hook in hooks {
            let _ = UnhookWinEvent(hook);
        }
        if let Some(h) = hidden {
            tray::remove(h);
            let _ = DestroyWindow(h);
        }
    }
    debug!("win32 event loop stopped");
}

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    send(Event::Shutdown);
    // For close/logoff/shutdown Windows kills the process as soon as this
    // returns, so give the core thread a moment to restore hidden windows.
    if matches!(
        ctrl_type,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    ) {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    true.into()
}

/// Turns Ctrl+C and closing the console into [`Event::Shutdown`]. Call after
/// [`EventThread::spawn`].
pub fn install_ctrl_handler() {
    // SAFETY: registering a valid handler function.
    if let Err(e) = unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), true) } {
        warn!(error = %e, "could not install console control handler");
    }
}
