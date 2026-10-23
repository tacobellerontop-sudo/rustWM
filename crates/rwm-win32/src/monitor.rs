use rwm_core::{MonitorInfo, Rect};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

const MONITORINFOF_PRIMARY: u32 = 1;

pub(crate) fn rect_from(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

unsafe extern "system" fn collect(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed by `enumerate` below,
    // which outlives the synchronous EnumDisplayMonitors call.
    let out = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    out.push(hmonitor);
    true.into()
}

fn info(hmonitor: HMONITOR) -> Option<MonitorInfo> {
    let mut mi = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `mi` is a properly sized MONITORINFOEXW with cbSize set.
    let ok = unsafe { GetMonitorInfoW(hmonitor, &mut mi.monitorInfo) };
    if !ok.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: out-pointers are valid locals. On failure we keep 96 DPI.
    let _ = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let len = mi
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(mi.szDevice.len());
    Some(MonitorInfo {
        id: String::from_utf16_lossy(&mi.szDevice[..len]),
        rect: rect_from(mi.monitorInfo.rcMonitor),
        work_area: rect_from(mi.monitorInfo.rcWork),
        scale: dpi_x as f32 / 96.0,
        primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

/// All monitors, primary first, then left to right.
pub(crate) fn enumerate() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which lives across the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }
    let mut monitors: Vec<_> = handles.into_iter().filter_map(info).collect();
    monitors.sort_by_key(|m| (!m.primary, m.rect.x, m.rect.y));
    monitors
}
